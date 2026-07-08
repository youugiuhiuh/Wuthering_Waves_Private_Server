# Task 2 Report: Create shared/state_ops.rs — state operation intercepts

## Status: DONE_WITH_CONCERNS

## Commits
- `b2cf5565` feat(aegis): add shared state_ops for lang/set_timeout/warp intercepts

## Files Created / Modified
| Action | File | Description |
|--------|------|-------------|
| CREATE | `src/shared/state_ops.rs` | `intercept()` + `handle_set_timeout`/`handle_lang`, plus adapted TDD test module |
| MODIFY | `src/shared/mod.rs` | Added `pub(crate) mod state_ops;` |
| MODIFY | `src/lib.rs` | Added `extern crate self as aegis;` and `pub mod app; pub mod bootstrap;` (see Concerns) |
| MODIFY | `src/bootstrap.rs` | `MatrixSetupConfig` → `pub`; added `#[allow(clippy::new_without_default)]` on `ConfigValidator` |
| MODIFY | `src/app/state.rs` | 4× `#[expect(dead_code)]` → `#[allow(dead_code)]` |

## TDD Flow (followed)
1. Wrote test-only `state_ops.rs` (test module referencing `intercept`) + declared module in `mod.rs`.
2. Ran `cargo test shared::state_ops` → FAIL: `cannot find function intercept` (and `cannot find app/bootstrap in crate`).
3. Added full implementation (`intercept`/`handle_set_timeout`/`handle_lang`) per brief Step 2.3.
4. Added `pub(crate) mod state_ops;` to `mod.rs`.
5. Confirmed PASS: `intercept_set_timeout_persists` green.
6. Ran full suite + `cargo fmt` + `cargo clippy -- -D warnings` → all clean.
7. Committed (Rust sources only; `.superpowers` docs left unstaged).

## Test adaptation (per brief's KEY ADJUSTMENT)
- Local `NoopExecutor` implementing `SelfDestructExecutor` (`Box::pin(async { Ok(()) })`).
- `Arc::new(MockBotAdapter::new())` for the `Arc<dyn BotAdapter>` field.
- `MessageId`/`TargetId` imported from `crate::adapters::common` (not from `shared::types`).
- 504+ existing tests preserved (545 passed total).

## Verification
- `cargo fmt` — clean
- `cargo clippy -- -D warnings` — clean
- `cargo test` — 545 passed, 0 failed (≥ 504 requirement met)

## Shared-layer constraint compliance
- `state_ops.rs` uses only the `BotAdapter` trait and `crate::app::state::AppState` / `crate::bootstrap::BotSettings` types — no platform-specific types.

## Concerns
1. **Baseline was not actually compiling.** The committed lib (`lib.rs` at `2aacff95`) does NOT compile from a clean build: `src/adapters/telegram/handlers/callback.rs` (part of the lib) references `crate::app` / `crate::bootstrap`, which were only declared in the binary (`main.rs`), not the lib. A prior "507 tests pass" claim relied on stale `target/` cache. To satisfy the global `cargo test`/`cargo clippy` constraint, `lib.rs` had to declare `app`/`bootstrap`.
2. **`extern crate self as aegis;`** added to `lib.rs` so the `aegis::`-prefixed paths inside `app`/`bootstrap` (written for the binary crate) resolve when those modules are compiled as part of the lib. This is the minimal enabling change.
3. **Scope expansion beyond brief.** To make `clippy -D warnings` pass after pulling `app`/`bootstrap` into the lib, I had to fix pre-existing lint debt in `bootstrap.rs` (private_interfaces, new_without_default) and `app/state.rs` (unfulfilled `#[expect(dead_code)]` → `#[allow]`). These were never checked before because the lib didn't compile. Left `callback.rs` untouched as instructed.
4. The `a_warp_add_input` branch and `handle_lang` keep their Telegram-specific system operations out of shared layer, matching the brief.

## Files NOT Modified (per task constraints)
- `src/adapters/telegram/handlers/callback.rs` — left untouched.
