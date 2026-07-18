# Aegis Command Lifecycle Design

**Date:** 2026-07-18
**Status:** Approved design

## Goal And Evidence

Repair `rust/aegis/src/core/cmd_async.rs`. Its timeout wrappers currently drop
the command future without proving that the child, descendants, or output tasks
have terminated.

## Scope

Keep the public command helpers unless a small signature change is required for
correct cleanup. On Linux, spawn each command in its own process group. Drain
stdout and stderr concurrently into bounded diagnostic tails.

## Lifecycle

Normal completion waits for child status and both drain tasks. On timeout:

1. Send `SIGTERM` to the process group.
2. Wait a short bounded grace period.
3. Send `SIGKILL` if the group remains alive.
4. Reap the child and resolve both output tasks.
5. Return one timeout error containing bounded output and any cleanup failure.

The function never reports timeout while a known child or pipe task remains
unresolved. Diagnostic buffering retains only a fixed tail, so noisy commands
cannot grow memory without bound.

## Failure Policy

Spawn failure creates no cleanup obligation. Signal, wait, and pipe failures are
combined with the original timeout rather than replacing it. Unsupported
platform behavior is explicit; Linux guarantees are not presented as portable.

## Tests And Acceptance

- A timed-out shell child and its descendant both disappear.
- stdout and stderr are drained concurrently without deadlock.
- output larger than the diagnostic cap is truncated to the tail.
- normal exit status and existing helper semantics are preserved.
- timeout does not return before process-group termination and reaping complete.

## Residual Risk

Processes that deliberately escape the created process group are outside this
boundary; the test proves control of ordinary descendants.
