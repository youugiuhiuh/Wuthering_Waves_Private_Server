# Task 4 Report: Migrate Menu Handler to Shared Layer

## Status: ✅ Complete

## Changes Made

### 1. Created `src/shared/handlers/menu.rs`
Shared handler with adapter-agnostic business logic:
- `send_main_menu(adapter: &dyn BotAdapter, target: &TargetId)` — sends the main menu
- `handle(event: &CallbackEvent) -> HandlerResult` — handles all menu callback data patterns (m_main, m_ops_center, m_settings, m_net_opt, m_security, m_sys_cmd, m_mon, m_usr, m_danger, m_session_timeout, set_timeout:, a_wwps_core_menu, a_wwps_box_menu, a_wwps_box_restart, a_wwps_box_status, a_wwps_core_latest, a_wwps_core_tags, wwps_core_tag:, a_geo_menu, a_wwps_core_menu, a_wwps_box_menu)

Key conversion patterns applied:
- `teloxide::Bot` → `&dyn BotAdapter`
- `ChatId` → `&TargetId`
- `InlineKeyboardButton::callback(t, d)` → `InlineButton { text, data }`
- `InlineKeyboardMarkup::new(rows)` → `Markup { buttons }`
- `bot.edit_message_text(...).parse_mode().reply_markup()` → `adapter.edit_message(target, msg_id, MessageContent { text, markup })`
- `bot.send_message(...).parse_mode().reply_markup()` → `adapter.send_message(target, MessageContent { text, markup })`
- `bot.answer_callback_query(id).text(t)` → `adapter.answer_callback(target, &callback_id, Some(t))`
- `ResponseResult<()>` → `anyhow::Result<()>` / `HandlerResult`
- `ParseMode::Html` removed (format-agnostic)

### 2. Updated `src/shared/types.rs`
Added `session_timeout_secs: u64` to `CallbackEvent` for the menu handler to read current timeout.

### 3. Updated `src/shared/mod.rs`
- Changed `pub(crate) mod handlers` → `pub mod handlers` (needed by binary crate)
- Changed `pub(crate) mod types` → `pub mod types`

### 4. Updated `src/shared/handlers/mod.rs`
- Uncommented `pub(crate) mod menu` → `pub mod menu`
- Changed menu dispatch branch from `Ok(Some(HandlerAction::Done))` to `Ok(Some(menu::handle(event).await?))`

### 5. Updated `src/adapters/telegram/handlers/menu.rs` (compat wrapper)
Converted from teloxide-direct implementation to thin compat wrapper:
- For `set_timeout:*`: persists timeout to state/disk before delegating
- Constructs `CallbackEvent` from `CallbackContext`
- Converts `CallbackContext.q.id` → `event.callback_id`, `CallbackContext.msg_id` → `MessageId(string)`, `CallbackContext.chat_id` → `TargetId(string)`
- Converts shared `HandlerAction` back to telegram `HandlerAction`
- Delegates to `aegis::shared::handlers::menu::handle(&event)`

### 6. Updated `src/main.rs`
- Changed `menu::send_main_menu(bot, msg.chat.id)` → `aegis::shared::handlers::menu::send_main_menu(&*state.adapter, &target)` with error conversion
- Removed unused `use handlers::menu` import

### 7. Updated `src/lib.rs`
Added `pub(crate) mod utils` to make `format_duration_human` accessible from the shared handler.

## Verification
- `cargo fmt` — clean
- `cargo check` — clean (only pre-existing unused-code warnings)
- `cargo test --lib` — 390 passed, 0 failed
- `cargo test --bin aegis` — 71 passed, 0 failed
