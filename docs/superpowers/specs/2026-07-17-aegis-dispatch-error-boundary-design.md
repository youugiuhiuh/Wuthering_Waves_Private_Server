# Aegis Dispatch Error Boundary Design

## Summary

Introduce the first vertical slice of Aegis's error-handling migration: a typed
error envelope for synchronous event dispatch and one shared transport boundary
for Telegram, Matrix, and Discord.

Unexpected dispatch failures will no longer be silently discarded. Aegis will
log the complete error chain with non-sensitive event context and send the
administrator a localized generic error containing a correlation event ID.

## Goals

- Preserve and surface every synchronous `dispatch_event` failure.
- Give the administrator a safe event ID that can be correlated with logs.
- Apply identical boundary behavior to Telegram, Matrix, and Discord.
- Preserve existing successful behavior and current handler signatures where
  possible.
- Introduce no new dependency and retain the existing `log` ecosystem.

## Non-Goals

- Converting all core, handler, or adapter errors to typed variants.
- Supervising detached background tasks.
- Migrating from `log` to `tracing`.
- Refactoring callback routing, platform identity types, or scheduler startup.
- Fixing unrelated swallowed errors inside individual operations.

## Architecture

The transport adapters remain responsible for receiving platform events. They
delegate application work to `dispatch_event`, then pass its result through a
common transport-boundary function.

```text
Telegram / Matrix / Discord
            |
            v
       dispatch_event
            |
            v
   handle_dispatch_result
       |             |
       | success     | failure
       v             v
     return       log full chain
                  with event ID
                       |
                       v
                 send localized
                 generic message
                 with event ID
```

`dispatch_event` remains independent of logging, localization, and transport
notification policy. The shared boundary owns those concerns.

## Components

### DispatchError

Add a thin application error envelope for dispatch failures. The initial slice
models unexpected failures as an internal dispatch error and preserves the
underlying source chain.

It supports conversion from `anyhow::Error`, allowing existing handlers to keep
using `?` without a broad signature migration. It contains no localized text,
platform-specific types, or user-facing details.

The type may gain stable categories such as `InvalidInput`, `Unauthorized`, or
`Unavailable` in later slices. Those categories are deliberately excluded now
because this slice has no behavior that needs them.

### Dispatch Event Context

Derive a small, immutable context value from each `BotEvent`. It contains only:

- platform;
- event kind (`message`, `command`, or `callback`);
- user ID;
- target ID.

It must not include message text, command arguments, callback payloads, file
metadata, tokens, TOTP values, security-file data, configuration values, or
other user-controlled content.

### Shared Transport Boundary

The shared boundary accepts the application state, event context, and dispatch
result.

On success, it performs no action. On failure, it:

1. Generates a short opaque event ID using standard-library process-local state.
2. Logs the event ID, platform, event kind, user ID, target ID, and complete
   source chain at error level.
3. Resolves the existing `internal_error` localization key.
4. Sends a new message containing that generic text and the event ID.
5. If notification fails, logs that secondary failure once and stops.

The boundary always uses `BotAdapter::send_message`. It does not retry a failed
callback answer or message edit, and it never recursively reports notification
failures.

## User-Facing Message

Reuse the existing `internal_error` translations and append the opaque ID at the
boundary. For example:

```text
❌ Internal error. Check server logs.
Event ID: a1b2c3d4
```

The event ID carries no encoded error, identity, or platform information. It is
only a correlation key. No internal error message or source-chain text is sent
to the platform.

## Runtime Flow

1. A platform adapter normalizes an incoming update into `BotEvent`.
2. The platform entry point calls `dispatch_event`.
3. `dispatch_event` authenticates and routes the event as it does today.
4. A successful result ends without additional I/O.
5. A failed result crosses the shared transport boundary.
6. The boundary records the complete diagnostic chain and safe event metadata.
7. The boundary sends one localized generic notification with the event ID.
8. A notification failure is logged and not retried.

## Error Policy

- Lower layers continue returning existing `anyhow` errors in this slice.
- `DispatchError` establishes the application boundary without erasing sources.
- `dispatch_event` propagates errors and never decides how they are presented.
- Platform entry points stop discarding synchronous dispatch results.
- Full diagnostics remain server-side; platform messages remain generic.
- Secondary notification errors never replace or obscure the original error in
  logs.

## Testing Strategy

Implementation follows RED, GREEN, REFACTOR.

Unit and boundary tests will verify:

- conversion from `anyhow::Error` preserves the source chain;
- message, command, and callback events yield the expected safe event context;
- successful dispatch results do not send an error message;
- failed dispatch results send exactly one localized message with an event ID;
- a failed error notification does not retry, recurse, or panic;
- Telegram, Matrix, and Discord entry points all use the shared boundary.

Existing authentication, self-destruct, integration, and CLI tests must remain
green.

## Verification

Run from `rust/aegis`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Acceptance Criteria

- No synchronous platform call site silently discards a `dispatch_event` error.
- Every unexpected dispatch failure receives one correlation event ID.
- Logs correlate that ID with platform, event kind, user ID, target ID, and the
  complete error chain.
- The administrator receives only localized generic text and the event ID.
- Notification failure produces no retry loop or panic.
- Existing success paths behave unchanged.
- No dependency is added.
- Detached task supervision remains explicitly deferred to a later slice.

## Follow-Up Slices

After this slice is proven, later designs may address destructive-operation
errors, typed adapter errors, Sing-box and Xray error types, background-task
supervision, startup severity classification, and broader core error cleanup.
