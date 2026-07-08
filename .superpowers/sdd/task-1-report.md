# Task 1 Report: Shared Module Infrastructure & BotAdapter Trait Extension

## What was implemented

1. **`src/shared/mod.rs`** — Module declaration with `pub mod types` and re-export
2. **`src/shared/types.rs`** — `PlatformCapabilities` struct with `Default` derive, `telegram()`, `discord()`, `matrix()` constants
3. **`src/lib.rs`** — Added `pub mod shared;`
4. **`src/adapters/common/trait.rs`** — Imported `PlatformCapabilities`, added 3 new methods:
   - `answer_callback(callback_id, text)` — default impl returns `Ok(())`
   - `download_file(file_id)` — required, returns `Result<Vec<u8>>`
   - `capabilities()` — required, returns `PlatformCapabilities`
5. **TelegramAdapter**: Implemented all 3 — `answer_callback` uses teloxide's `answer_callback_query`, `download_file` uses `get_file` + `download_file`, `capabilities` returns `PlatformCapabilities::telegram()`
6. **DiscordAdapter**: `answer_callback` no-op, `download_file` uses `reqwest::get`, `capabilities` returns `PlatformCapabilities::discord()`
7. **MatrixAdapter**: `answer_callback` no-op, `download_file` uses matrix-sdk's `media().get_media_content()`, `capabilities` returns `PlatformCapabilities::matrix()`
8. **RoutingAdapter**: Delegated all 3 to `self.primary`
9. **Test stubs**: Updated 3 test-only `BotAdapter` impls (scheduler `TestAdapter`, destruct_flow `MockAdapter`, state `MockAdapter`)

## Testing

- `cargo fmt` ✓
- `cargo clippy -- -D warnings` ✓ (0 warnings)
- `cargo test` ✓ (446 tests: 375 lib + 71 bin + integration; 0 failed, 1 pre-existing ignore)

## Files changed

| File | Change |
|------|--------|
| `src/shared/mod.rs` | **Created** |
| `src/shared/types.rs` | **Created** |
| `src/lib.rs` | Added `pub mod shared` |
| `src/adapters/common/trait.rs` | Added import + 3 methods |
| `src/adapters/common/routing.rs` | Delegated 3 methods |
| `src/adapters/telegram/adapter.rs` | Implemented 3 methods |
| `src/adapters/discord/adapter.rs` | Implemented 3 methods |
| `src/adapters/matrix/adapter.rs` | Implemented 3 methods |
| `src/app/destruct_flow.rs` | Added stubs to test MockAdapter |
| `src/app/state.rs` | Added stubs to test MockAdapter |
| `src/core/system/scheduler/mod.rs` | Added stubs to test TestAdapter |

## Self-review findings

- **Design choice**: `answer_callback` has a default `Ok(())` so mockall won't generate `expect_answer_callback` — correct per requirements. `capabilities` and `download_file` are required and will be auto-mocked.
- **Matrix `download_file`**: Uses `room.client()` to access the media API. For encrypted files, decryption happens automatically via matrix-sdk.
- **Discord `download_file`**: Uses `reqwest::get` directly on the file URL — no serenity-specific API needed.
- **No breaking changes**: All existing tests pass without modification to their mock expectations (new methods are never called in existing test code).

## Concerns

None.
