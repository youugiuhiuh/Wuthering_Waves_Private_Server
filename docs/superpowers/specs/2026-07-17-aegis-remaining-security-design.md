# Aegis Remaining Security Design

**Date:** 2026-07-17
**Status:** Approved design, implementation plans ready
**Scope:** The four unresolved security findings in the Aegis Rust audit

## Goal

Close the remaining identity, software-supply-chain, sensitive-file, and
self-destruct risks without mixing unrelated stability or performance work into
the same changes.

## Source Evidence

- `docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`
- `rust/aegis/src/shared/types.rs`
- `rust/aegis/src/app/state.rs`
- `rust/aegis/src/core/singbox/installer.rs`
- `rust/aegis/src/core/security/crypto.rs`
- `rust/aegis/src/main/matrix.rs`
- `rust/aegis/src/core/network/warp_api.rs`
- `rust/aegis/src/core/xray/reality.rs`
- `rust/aegis/src/shared/dispatch.rs`
- `rust/aegis/src/shared/destruct.rs`
- `rust/aegis/src/core/security/self_destruct.rs`

## Delivery Model

This design is implemented as four independent security changes, in this order:

1. Namespaced principals and administrator migration.
2. Secure sensitive-file creation and replacement.
3. Verified Sing-box installation.
4. Principal-bound self-destruct state machine.

Each change receives its own TDD implementation plan, isolated worktree, review,
and pull request. A roadmap document links the four plans and records their
status. No pull request spans all four trust boundaries.

## Global Security Invariants

- Every security decision is fail closed. There is no compatibility bypass.
- Platform identity is part of identity; a numeric ID alone is never a principal.
- Authentication, failure counters, pending flows, and destructive operations
  are bound to the initiating principal.
- Untrusted network content is size-bounded, verified, and structurally checked
  before it can affect an installed binary or service.
- Sensitive files are never created with a permissive mode and tightened later.
- Symlinks and non-regular files are rejected at sensitive destinations.
- User-facing errors contain only a generic message and event ID. Secrets,
  authentication material, and full hashes never enter user messages or logs.
- Existing data or binaries remain intact when a security check fails.
- No new dependency is added unless the standard library and existing
  dependencies cannot implement the required invariant safely.

## 1. Namespaced Principals

### Model

Introduce a validated, hashable, serializable `Principal` containing:

- `platform`: Telegram, Discord, or Matrix.
- `subject`: the canonical platform-native user identity.

Telegram and Discord subjects are canonical decimal strings. Matrix subjects
are complete Matrix IDs, including the homeserver. Matrix localparts are not
accepted as identities. Empty subjects, whitespace variants, and values that do
not conform to the selected platform are rejected at the adapter boundary.

`Platform` receives a stable serialized representation and the traits required
to use it in maps and sets. Wire compatibility must not depend on enum ordinal
values.

### Event And State Flow

`MessageEvent`, `CommandEvent`, and `CallbackEvent` carry a `Principal`
directly. `BotEvent::principal()` returns that value without parsing or
fallback. The current parse-to-`i64` and fallback-to-zero behavior is removed.

`AppState` stores administrators as a set of principals. Sessions, failed
authentication attempts, pending security-file input, and every destructive
flow use `Principal` or a composite `Principal + TargetId` key. Equal subject
strings from different platforms never share state.

### Migration

Legacy numeric administrator fields are migrated at startup before `AppState`
is constructed:

- A legacy Telegram administrator maps to a Telegram principal only when the
  enabled platform configuration makes that mapping unique.
- A configured Discord administrator maps to a Discord principal.
- Matrix administration requires a complete configured Matrix ID; a numeric
  value or localpart is never inferred.
- Ambiguous, invalid, or incomplete migration stops startup with an actionable
  configuration error.
- The new configuration is persisted atomically before it is used.
- In-memory sessions and failed-attempt records are intentionally not migrated.

After successful migration, normal startup no longer reads legacy fields. There
is no first-login binding or indefinite dual-format compatibility layer.

### Acceptance Criteria

- Same numeric subject on two platforms produces unequal principals.
- Matrix users with equal localparts on different homeservers are unequal.
- Invalid callback identity cannot become principal zero or another identity.
- Cross-platform sessions and failure counters remain isolated.
- Unique legacy configurations migrate; ambiguous configurations refuse startup.

## 2. Secure Sensitive-File Writes

### Boundary

Add one focused `secure_fs` module with two responsibilities:

- Ensure a private directory exists with mode `0700` and valid ownership/type.
- Create or atomically replace a sensitive regular file with mode `0600`.

The module is used only for the audit-listed keys, tokens, WARP account files,
and Reality seed paths. General configuration persistence is outside this
design.

### Replacement Protocol

For each write:

1. Validate the parent directory, destination type, and ownership.
2. Reject symlinks, non-regular destinations, and ownership mismatches.
3. Create a unique same-directory temporary file with `create_new`,
   `O_NOFOLLOW`, and mode `0600`.
4. Write the complete value and call `sync_all` on the temporary file.
5. Revalidate the destination immediately before replacement.
6. Rename the temporary file over the destination atomically.
7. Synchronize the parent directory.

Any failure removes only the temporary file and preserves the previous valid
destination. The implementation does not silently chmod, chown, or replace a
suspicious existing object.

### Acceptance Criteria

- New private directories are `0700`; new sensitive files are `0600` at birth.
- Symlinks, non-regular files, and incorrect owners are rejected.
- Injected write, sync, validation, and rename failures preserve old contents.
- Concurrent replacement cannot redirect a write outside the validated parent.

## 3. Verified Sing-box Installation

### Release Trust

The release source is fixed to `SagerNet/sing-box`. The installer follows the
repository's latest GitHub release and accepts only the exact Linux asset name
computed from the supported architecture and release tag.

Metadata is fetched through the existing fixed GitHub API client. Optional
`GITHUB_TOKEN` authentication is restricted to `api.github.com`. Asset downloads
reuse the existing unauthenticated asset client and exact per-redirect host
allowlist. Only `browser_download_url` is accepted.

The selected asset must include a valid GitHub API SHA256 digest. This digest is
integrity evidence from the same GitHub trust domain, not an independent
publisher signature. Missing or malformed digest data stops installation.

### Download And Extraction

- Enforce both declared `Content-Length` and streamed-byte limits.
- Download into a unique private temporary directory (`0700`) and file (`0600`).
- Verify the complete archive SHA256 before parsing it.
- Inspect each tar entry instead of extracting the archive wholesale.
- Reject absolute paths, parent traversal, symlinks, hard links, device nodes,
  unexpected binary paths, duplicate binaries, and expanded-size overflow.
- Accept exactly one regular `sing-box` binary at the expected release path.

The candidate binary is staged beside the installed destination, assigned mode
`0755`, and executed with `sing-box version`. Its reported version must equal the
release tag. Only then may an atomic replacement occur. Service creation or
restart happens after replacement. Any prior failure leaves the current binary
and service untouched and removes the private temporary directory.

### Acceptance Criteria

- Repository, asset selection, API origin, token boundary, and redirect hosts
  cannot be changed at runtime.
- Missing digest, size overflow, hash mismatch, malicious archive entry,
  duplicate candidate, or version mismatch prevents replacement.
- Every failed verification test asserts that the old binary and service state
  remain unchanged.

## 4. Principal-Bound Self-Destruct

### Preconditions And State

Self-destruct is unavailable until a security-file hash is configured. Starting
the flow also requires a current administrator principal and a recently
authenticated session.

State is keyed by `Principal + TargetId` and records:

- Current typed step.
- Absolute deadline set once at flow creation.
- Total failed attempts.
- Accepted TOTP counters.
- One-time final-confirmation nonce.
- Terminal state: cancelled, expired, locked, executing, succeeded, or failed.

The absolute deadline is five minutes. Failed input never extends it. The first
three authentication failures are recorded and apply delays of one, two, and
four seconds. A fourth authentication submission is rejected without verifying
its value and atomically locks the flow.

### Authentication And Callbacks

Both accepted TOTP values must come from different counters. An accepted counter
is consumed immediately and cannot be reused by another step or flow. The
security file must match the configured hash.

Every message and callback independently verifies the principal, target,
administrator status, recent session, deadline, and expected state transition.
Cancel and confirm callbacks are not processed before authorization. Final
confirmation consumes a random nonce exactly once and atomically transitions
the flow to `Executing`; concurrent or replayed confirmations cannot execute the
destructive operation twice.

The executor is supervised and returns an observable result. Errors are logged
through the shared event boundary without secret material. Fire-and-forget
execution is removed.

### Acceptance Criteria

- Failures do not extend the deadline; the fourth submission locks the flow
  without verification.
- A consumed TOTP counter cannot be replayed.
- Cross-principal and cross-target cancel or confirm events have no effect.
- Expired or no-longer-authorized sessions cannot advance the flow.
- Missing security-file configuration prevents flow creation.
- Concurrent final confirmations invoke the executor exactly once.

## Error And Recovery Policy

- Security validation errors are not retried with weaker behavior.
- Startup migration errors identify the configuration field and expected format
  without printing credentials.
- Network and archive errors occur before installation side effects.
- Sensitive-file errors preserve the previous file and report the failed stage.
- Self-destruct failures enter a terminal state and require a new authenticated
  flow; they do not resume from a partially trusted state.

## Test And Review Gates

Each implementation plan follows RED, GREEN, REFACTOR and includes focused unit
tests, side-effect assertions, and integration tests for its trust boundary.
Every phase must pass:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

The complete security program also requires:

- Dependency vulnerability scanning with recorded tool/version/output.
- Malicious archive and symlink fixture tests.
- Concurrent identity and self-destruct state stress tests.
- Review of logs for token, TOTP, security-file, and full-hash disclosure.
- Specification compliance review followed by code-quality review per phase.

Unavailable scanning tools or environment-dependent checks are recorded as
unsatisfied gates, not reported as passing. Critical or Important findings block
the next phase.

## Audit Closure

After each verified merge, update only the corresponding security finding in
`docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`
with commit, tests, and residual-risk evidence. Stability and performance
findings retain their current status.

## Non-Goals

- Child-process timeout and process-group lifecycle changes.
- General configuration locking or atomic persistence beyond the migration and
  audit-listed sensitive files.
- Port allocator concurrency.
- Upgrade rollback for Aegis or Xray.
- SNI, TLS probe, cache, or batch performance work.
- A generic plugin, mirror, repository, or trust-policy framework.
- Supporting arbitrary third-party Sing-box repositories.
