# Aegis Bulk Configuration Deletion Error Design

## Context

Aegis currently reports successful bulk configuration deletion even when some
files were not removed. Xray and Sing-box implement similar deletion flows, but
they suppress failures in both core managers and handlers:

- Xray counts discovered files rather than successful removals in its delete-all
  operation.
- Xray handlers duplicate deletion logic and ignore listing, metadata, removal,
  and reload failures.
- Sing-box suppresses individual removal and metadata failures.
- Sing-box handlers expose raw internal errors to users instead of using the
  shared dispatch error boundary.

The first error-handling slice established a shared dispatch boundary that logs
full server-side context and sends users a generic localized error with an event
ID. This slice makes bulk configuration deletion propagate accurate failures to
that boundary.

## Goal

For Xray and Sing-box bulk configuration deletion, report success only when all
target-file processing and any required service reload complete without error.
On failure, preserve the actual deleted count and detailed causes while exposing
only the existing safe dispatch-boundary message to users.

## Scope

This slice covers:

- Xray delete-all operations.
- Xray protocol-filtered bulk deletion.
- Xray delete-by-count operations.
- Sing-box delete-all operations.
- Sing-box delete-by-count operations.
- The Xray and Sing-box handlers that invoke those operations.

This slice does not cover:

- Single-configuration deletion.
- Sing-box firewall commands that currently ignore exit status.
- Port allocator release failures.
- `ops::handle_reload` false-success behavior.
- Installer, firewall synchronization, or detached-task supervision.
- A global replacement of `anyhow` or adoption of another error library.
- New partial-success user-facing copy.
- Rollback of files that were already deleted successfully.

## Architecture

Add a focused shared core module, `core/config_delete.rs`, containing the common
error contract and result aggregation:

- `BulkDeleteError` describes discovery, partial-operation, and reload failures.
- `FileDeleteFailure` preserves the affected path, failure stage, and original
  error.
- `DeleteStage` distinguishes `Inspect`, `Prepare`, and `Remove` failures.
- A small tracker records the target count, successful removals, per-file
  failures, and an optional reload failure.

The shared module does not become a generic asynchronous deletion engine. Xray
and Sing-box keep local orchestration because Sing-box has protocol-specific
Hysteria cleanup before file removal. Each manager uses a private helper with an
injectable reload function so filesystem behavior can be tested without invoking
real services.

Public bulk-deletion APIs return:

```rust
Result<usize, BulkDeleteError>
```

`Ok(n)` means exactly `n` files were removed and any required reload succeeded.
An error retains the real successful count and all recorded failures.

## Operation Flow

Every bulk operation follows this sequence:

1. List candidate configuration files. A listing failure returns a discovery
   error before any deletion occurs.
2. For delete-by-count, inspect modification times and sort valid candidates
   from oldest to newest. Metadata failures are recorded as `Inspect` failures;
   remaining sortable files continue through selection.
3. Process selected files independently:
   - Xray attempts file removal directly.
   - Sing-box identifies Hysteria configurations, parses their port information,
     and runs the existing cleanup before removal.
   - A Sing-box preparation failure is recorded as `Prepare`, and that file is
     not deleted. This avoids losing the configuration needed to identify
     associated rules.
   - A removal failure is recorded as `Remove`.
   - Processing continues after any per-file failure.
4. If at least one file was removed successfully, reload the corresponding core
   exactly once. This keeps runtime state aligned with the changed filesystem
   even when other files failed.
5. Return `Ok(actual_deleted)` only if there are no recorded failures and reload
   succeeded. Otherwise return `BulkDeleteError` containing the deleted count,
   per-file failures, and reload failure when present.

There is no rollback. File deletion and service reload cannot be made safely
transactional, so successful removals remain in place.

For delete-by-count, metadata failures do not silently reduce observability. The
operation records them, then selects up to the requested number from candidates
that can be ordered. The final result is an error even if those selected files
were removed successfully.

## Handler Integration

Xray handlers currently bypass `ConfigManager` for protocol-filtered and
count-limited deletion. Move those operations behind the manager boundary so all
Xray bulk deletion uses the same typed semantics. Handlers must not call
`fs::remove_file` directly.

Both handler modules follow one rule:

- Await the core operation with `?`.
- Send the existing success callback only after receiving `Ok(actual_deleted)`.
- Do not catch `BulkDeleteError` to render `error.to_string()`.

The existing conversion into `anyhow::Error` at the handler boundary allows the
error to reach `DispatchError`. The shared dispatch boundary then logs the full
chain and sends the generic localized `internal_error` message plus event ID.
Paths and infrastructure details never appear in user-visible messages.

## Error Semantics

The error display should summarize operation type, target count, successful
count, failed count, and whether reload failed. Detailed server-side formatting
must retain each failed path, stage, and original error. At least one underlying
source must remain available through Rust's `Error::source`; all failure records
remain inspectable on the typed error.

When file failures and reload failure occur together, neither category may
replace or hide the other. The operation reports one structured error containing
both.

No new dependency is required. The project already uses `thiserror`, `anyhow`,
and `tempfile`.

## Testing

Tests use temporary directories and injected reload functions. They must not run
Xray, Sing-box, iptables, systemd, or other host-level operations.

Required core coverage:

- All selected files delete successfully and return the exact count.
- A missing or duplicate path records a `Remove` failure while later files are
  still processed.
- Delete-by-count records an `Inspect` failure and continues with sortable files.
- Zero successful removals do not invoke reload.
- One or more successful removals invoke reload exactly once.
- Partial removal failure still invokes reload when at least one removal
  succeeded.
- Simultaneous file and reload failures are both retained.
- A malformed Hysteria configuration records `Prepare`, remains on disk, and
  does not prevent processing other candidates.
- An empty candidate set returns `Ok(0)` without reload.

Handler acceptance checks:

- Xray bulk handlers no longer call `fs::remove_file`.
- Covered paths no longer use `unwrap_or_default`, `unwrap_or(0)`, or `let _ =`
  to suppress listing, deletion, or reload failures.
- Sing-box bulk handlers no longer send `error.to_string()` to users.
- Existing success messages are sent only after complete success.

Final verification commands from `rust/aegis`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

The known xray-core external-binary test failure must be compared with the
current `main` baseline and reported separately; it is not caused by this slice.

## Success Criteria

- Returned success counts equal actual successful removals.
- No bulk deletion or reload failure in scope is silently discarded.
- Partial deletion always returns a structured error.
- Runtime reload occurs once whenever at least one file changed.
- Internal paths and error details remain server-side.
- Xray and Sing-box handlers rely on the shared dispatch error boundary.
- No unrelated destructive-operation behavior changes.
