# Design: Decouple Telegram Dependencies from rust/aegis app Layer

**Date**: 2026-07-07
**Status**: Approved
**Mode**: Strict

## Goal

Remove platform-specific (Telegram) type dependencies from `app/batch_handler.rs` and
`app/destruct_flow.rs`. Business logic must be unified across platforms; only input
parsing and UI rendering differ per adapter ("platform has what platform has").

## Out of Scope

- `core/` business logic.
- `main.rs`, `main/runtime.rs` entry points (stay Telegram/Matrix for now).
- `Cargo.toml` feature flags / conditional compilation.
- Wiring Discord adapter into runtime.

## Files Changed

| File | Action |
|------|--------|
| `src/app/batch_handler.rs` | Modify: replace `teloxide::types::ChatId` with `&TargetId` |
| `src/app/destruct_flow.rs` | Modify: add platform-agnostic `DestructInput`/`DestructOutput`/`handle_input`; remove Telegram handler code |
| `src/adapters/telegram/handlers/destruct.rs` | **New**: Telegram-specific destruct UI handler |
| `src/adapters/telegram/handlers/message.rs` | Modify: call new `destruct::handle_message_flow` |
| `src/adapters/telegram/handlers/callback.rs` | Modify: call new destruct callback handler |
| `src/adapters/telegram/handlers/mod.rs` | Modify: expose `destruct` module |

## Design Details

### 1. `app/batch_handler.rs`

**Remove** `use teloxide::types::ChatId;`

**Change signature:**
```rust
pub async fn send_singbox_batch_result(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,          // was chat_id: ChatId
    protocol_name: &str,
    result: &BatchCreationResult,
) -> anyhow::Result<()>
```

Internally use `target` directly. Tests use `TargetId("1".to_string())`.

### 2. `app/destruct_flow.rs`

Keep (platform-agnostic business logic):
- `DestructStep`, `DestructMessageAction`, `MessageFlowOutcome`
- `process_destruct_message()`

Add (platform-agnostic I/O):

```rust
pub enum DestructInput {
    Text(String),
    File(Vec<u8>),
    Button(String),
}

pub enum DestructOutput {
    Prompt { text: String, markup: Option<Markup> },
    SendText(String),
    ExecuteSelfDestruct,
    Noop,
    InvalidState,
}

pub async fn handle_input(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: i64,
    input: DestructInput,
    now: Instant,
) -> anyhow::Result<(MessageFlowOutcome, Vec<DestructOutput>)>
```

Remove (move to Telegram adapter):
- `handle_message_flow`, `handle_callback_timeout`, `handle_callback_action`

### 3. `adapters/telegram/handlers/destruct.rs` (New)

Telegram-only: parse `Message`/`CallbackQuery` → `DestructInput`, call `handle_input`,
render `DestructOutput` via `Bot`. Keeps `teloxide::Bot`, `ChatId`, `InlineKeyboardMarkup`.

### 4. Caller Updates

- `message.rs`, `callback.rs`: import and call functions from the new `destruct.rs`.
- `mod.rs`: add `pub mod destruct;`.

## Testing

| Test | Location | Content |
|------|----------|---------|
| batch_handler unit | `app/batch_handler.rs` | Switch `ChatId` to `TargetId`, verify `MockBotAdapter` calls |
| destruct business logic | `app/destruct_flow.rs` | Keep existing tests; add `handle_input` tests |
| Telegram handler | Telegram adapter | Manual verification or basic mock of `Bot` |

## Risks

1. No circular imports between `crate::app::destruct_flow` and the new handler.
2. Callback data strings (`a_destroy_*`) become a protocol constant defined in
   `app/destruct_flow.rs`.
3. Future Matrix handler can map text commands to `DestructInput::Button` without
   touching app layer.
