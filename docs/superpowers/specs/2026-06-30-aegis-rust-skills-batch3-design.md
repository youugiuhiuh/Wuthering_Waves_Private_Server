# Aegis Rust Skills Batch 3: Error Handling Improvements

> Based on RUST_SKILLS_REVIEW.md Section 2 (Error Handling, CRITICAL).

## Status Summary

| Rule | Batch 1 Fix | Batch 2 Fix | Batch 3 Target |
|------|-------------|-------------|----------------|
| err-result-over-panic ❌ | firewalld.rs, bootstrap.rs partial | — | Remaining production `unwrap()` → `expect()` |
| err-no-unwrap-prod ❌ | Partial | — | Remaining production `unwrap()` → `expect()` |
| err-source-chain ⚠️ | — | — | `AppError` `#[source]` annotation |
| err-doc-errors ❌ | — | Core types (#Errors on BotAdapter, SecurityManager, TotpManager) | Remaining public Result functions |

## Approach

Three independent sub-tasks, no behavioral changes:

### Section 1: Production `unwrap()` → `expect()`

Convert low-risk production unwraps to `expect()` with Chinese semantic messages:

- `core/sni/selector.rs:146` — `pop().unwrap()` with preceding is_empty guard
- `core/system/operations.rs:216` — `periodic_config_path().unwrap()` on known Debian path
- `adapters/telegram/handlers/schedule/handle.rs:678` — `strip_prefix().unwrap()` pre-validated by router
- `core/security/firewall_scanner.rs:10,12` — Static Lazy Regex compilation
- `core/xray/port_allocator.rs:142` — Regex compilation (if production)

### Section 2: `#[source]` for AppError

In `core/error.rs`, add `#[source]` attribute to `AppError` variants that wrap inner errors to preserve the error chain:

- Currently: `Config(String)`, `Service(String)` etc. lose the source error
- Fix: Add `#[source]` where applicable, or document why it's intentionally omitted

### Section 3: `# Errors` doc gap fill

Scan all public `Result`-returning functions for missing `# Errors` sections. Likely gaps:
- `SystemMonitor` methods
- `SchedulerManager` methods
- Batch handler functions

## Files Modified

- `src/core/error.rs` — AppError `#[source]`
- `src/core/sni/selector.rs` — `expect()`
- `src/core/system/operations.rs` — `expect()`
- `src/adapters/telegram/handlers/schedule/handle.rs` — `expect()`
- `src/core/security/firewall_scanner.rs` — `expect()` on static Regex
- `src/core/xray/port_allocator.rs` — `expect()` on static Regex (if production)
- Plus any `# Errors` doc gaps found

## Non-Goals

- No behavioral logic changes
- No test code unwraps (tests can use `unwrap()`; the "no unwrap in prod" rule doesn't apply to tests)
- No unwraps in `#[cfg(test)]` modules or `#[test]` functions
- No API changes

## Verification

- `cargo clippy -D warnings` clean
- `cargo test` 446 passed, 0 failed
- `cargo fmt --check` clean
