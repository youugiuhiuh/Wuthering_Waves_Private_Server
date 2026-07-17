# Aegis Command Lifecycle Stability Design

**Date:** 2026-07-17

## Purpose

Prevent timed-out, failed, or cancelled system commands from surviving as
orphaned processes, retaining package-manager locks, or leaking unbounded
stdout/stderr into memory.

This is the first remediation slice from the Aegis stability audit. It is
deliberately limited to the common async command executor so later installer,
upgrade, scheduler, and state-transaction work can build on a reliable process
lifecycle boundary.

## Scope

Modify only:

- `rust/aegis/src/core/cmd_async.rs`

The following public functions retain their existing signatures and caller
semantics:

- `run_cmd_output`
- `run_cmd_status`
- `run_cmd_checked`
- `run_cmd_stream`

No caller migration, installer transaction, upgrade rollback, scheduler
change, or port-allocation change is included in this slice.

## Platform Assumption

Aegis manages Linux services and this implementation may use Unix process
groups and signals. The design does not add a cross-platform abstraction for
Windows.

No new dependency is required. The crate already depends on `libc` and Tokio.

## Architecture

The public API remains a thin layer over private lifecycle operations:

1. Build a `tokio::process::Command` with piped stdout and stderr.
2. Start the command in a new Unix process group.
3. Enable direct-child cleanup with `kill_on_drop(true)`.
4. Arm a private process-group guard immediately after spawn.
5. Drain stdout and stderr concurrently until both reach EOF.
6. Wait for the child without holding either output stream open.
7. Disarm the guard only after the child has been successfully reaped.

Output capture and streaming may use separate private drain implementations,
but process-group creation, timeout termination, and final reaping must share
one lifecycle policy.

## Process Group Guard

The private guard owns the process-group identifier and remains armed until
reaping is complete.

If the command future is dropped because its caller is aborted, a callback
panics, or an unexpected early return occurs, dropping the armed guard sends
`SIGKILL` to the entire process group. `kill_on_drop(true)` remains enabled for
the direct child.

The guard must not send a signal after a normal wait or completed timeout
cleanup. It is disarmed only after the child has been reaped.

## Normal Completion

### Captured output

`run_cmd_output` drains stdout and stderr concurrently. Each stream keeps only
its final 1,048,576 bytes.

If a stream exceeds that limit, the returned string starts with:

```text
[output truncated; showing last 1048576 bytes]
```

Truncation does not alter the exit status or turn a successful command into an
error. Conversion to text starts at a valid UTF-8 boundary and continues to
use lossy conversion for invalid source bytes.

### Streaming output

`run_cmd_stream` continues calling the existing synchronous callback for each
line received from either stream. EOF from one stream must not stop draining
the other stream. A read failure is propagated rather than treated as EOF.

The streaming path does not accumulate complete output. Any diagnostic tail it
retains must obey the same fixed memory bound.

### Status helpers

`run_cmd_status` continues returning only `ExitStatus`.

`run_cmd_checked` continues failing on a non-zero status and selecting stderr,
or stdout when stderr is empty, for diagnostic context. Because both sources
are bounded, its error message is bounded as well.

## Timeout Termination

When the configured timeout expires:

1. Send `SIGTERM` to the entire process group.
2. Allow a two-second grace period for the group leader to exit.
3. If it remains active, send `SIGKILL` to the process group.
4. Wait for and reap the direct child.
5. Complete or join output drain tasks so no pipe task remains detached.
6. Return a timeout error.

An `ESRCH` signal result means the target already exited and is not itself an
error. Other signal and wait errors are retained in the error chain.

Timeout errors identify the program, timeout duration, and whether forced KILL
was required. They do not include the full argument list because arguments may
contain credentials or other sensitive values.

## Other Failure Handling

### Spawn failure

Return an error containing the program name and the original spawn source.

### Output read failure

Terminate and reap the process group before returning the read error. Do not
silently convert the failure into an EOF condition.

### Termination or wait failure

Preserve the original timeout or read error and attach signal/wait failures as
additional context. Cleanup failures must never replace the initiating error.

### Caller cancellation

Dropping the command future must kill the entire process group through the
armed guard. The Tokio child remains configured for direct-child cleanup.

## Concurrency and Memory Guarantees

- stdout and stderr are always drained independently and concurrently.
- One stream reaching EOF cannot prevent the other stream from completing.
- Each captured stream retains at most 1 MiB plus a fixed truncation marker.
- No unbounded channel is introduced.
- No mutex or synchronous file operation is held across an await.
- All normal and timeout paths reap the direct child before returning.

## Test Design

Tests remain in the existing `cmd_async.rs` test module and follow TDD.

### Normal behavior

- Capture stdout on a successful command.
- Capture stderr on a successful command.
- Preserve non-zero exit status and checked-command diagnostics.

### Independent EOF handling

- Close stdout early, emit delayed stderr, and verify stderr is retained.
- Close stderr early, emit delayed stdout, and verify stdout is retained.

### Bounded output

- Produce more than 1 MiB on stdout and verify the truncation marker, bounded
  returned size, and final sentinel bytes.
- Repeat for stderr.
- Include a multibyte character across the truncation boundary and verify the
  returned string is valid UTF-8.

### Graceful timeout

- Start a shell process that records its PID, handles `SIGTERM`, and exits.
- Trigger timeout and verify the PID no longer exists before the function
  returns.

### Forced timeout

- Start a process that ignores `SIGTERM`.
- Verify the two-second grace path proceeds to `SIGKILL` and the PID disappears.

### Descendant cleanup

- Start a shell that records both its PID and a background `sleep` PID.
- Trigger timeout and verify neither process remains alive.

### Caller cancellation

- Spawn a task running a long-lived command and record parent and descendant
  PIDs.
- Abort the Tokio task and verify the complete process group disappears.

### Existing error behavior

- Preserve coverage for nonexistent programs, non-zero exits, streaming lines,
  and timeout errors.

All PID-based tests use temporary files and bounded polling. They must not
require root privileges or invoke production system-management commands.

## Acceptance Criteria

- Existing public command helper signatures do not change.
- Timed-out commands and their descendants are terminated and reaped.
- Aborted command futures leave no surviving process group.
- stdout and stderr remain independently drainable after the opposite stream
  reaches EOF.
- Captured output is bounded to 1 MiB per stream plus a fixed marker.
- Existing command-helper behavior remains covered and passing.
- `cargo fmt --check` passes.
- `cargo clippy --lib --all-features -- -D warnings` passes.
- Full library tests introduce no new failures beyond the documented external
  Xray binary environment failures.

## Explicit Non-Goals

- Rewriting all direct `tokio::process::Command` callers in the repository.
- Adding an injectable command-runner trait.
- Adding Windows process-tree support.
- Changing user-visible handler messages.
- Implementing installer or upgrade single-flight and rollback.
- Fixing configuration, scheduler, or port-allocation persistence.
