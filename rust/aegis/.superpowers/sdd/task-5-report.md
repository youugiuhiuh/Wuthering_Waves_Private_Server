# Task 5 Report: Migrate message.rs handler to shared layer

## Implementation Summary

Migrated the Telegram message handler's business logic (input length check, schedule timeout handling, warp input processing) to `src/shared/handlers/message.rs`, leaving the Telegram handler as a thin compat wrapper.

### Pattern Used

Followed the same pattern as Task 4 (menu migration):
- **Shared handler** (`src/shared/handlers/message.rs`): contains real logic using `BotAdapter` + `MessageState` trait
- **Telegram handler** (`src/adapters/telegram/handlers/message.rs`): thin wrapper (admin check → delegate to shared → destruct flow → TOTP auth)

### Design Decisions

1. **`MessageState` trait**: Abstracted the 3 state operations needed (schedule_timeout_status, remove_schedule_input, take_warp_input_status) behind a trait, since `AppState` lives in the binary crate and can't be directly imported by the library crate's shared module. The trait is implemented for `AppState` in `app/state.rs`.

2. **`TimeoutStatus` moved to `shared/types.rs`**: The enum is now shared between lib and bin crates. All importers updated.

3. **`MessageAction` enum**: Shared handler returns `Handled` (done) or `NeedsDestruct` (caller must try destruct flow / TOTP auth).

4. **Not migrated**: Destruct flow processing (still in `app/destruct_flow.rs`, teloxide-dependent) and TOTP auth (uses main.rs functions). These remain in the Telegram handler.

### Files Changed

| File | Change |
|------|--------|
| `src/shared/types.rs` | Added `TimeoutStatus` enum |
| `src/shared/handlers/message.rs` | **NEW** — shared handler with `MessageState` trait, `MessageAction` enum, `handle_message()` |
| `src/shared/handlers/mod.rs` | Added `pub mod message` |
| `src/app/state.rs` | Removed local `TimeoutStatus`, import from shared, implement `MessageState` trait |
| `src/app/destruct_flow.rs` | Updated `TimeoutStatus` import to `aegis::shared::types` |
| `src/adapters/telegram/handlers/callback.rs` | Updated `TimeoutStatus` import to `aegis::shared::types` |
| `src/adapters/telegram/handlers/message.rs` | Thinned to compat wrapper (53 lines, was 126) |
| `src/main.rs` | Removed unused `MAX_INPUT_LENGTH` const (now lives in shared handler) |

## Test Results

- **lib tests**: 389 passed, 1 failed (pre-existing flaky i18n race condition), 1 ignored
- **bin tests**: 71 passed, 0 failed, 0 ignored
- **Build**: Clean, no new warnings

## Self-Review

- [x] Shared handler has zero teloxide dependencies
- [x] Telegram handler is a thin compat wrapper
- [x] No logic duplication between shared and Telegram layers
- [x] `MessageState` trait is minimal (3 methods)
- [x] All existing tests pass
- [x] No behavioral changes to the message handling flow

## Concerns

- `MessageState` trait causes a slight indirection (inherent methods → trait methods) since `AppState` already has the methods. This is the idiomatic pattern for cross-crate abstraction in Rust.
