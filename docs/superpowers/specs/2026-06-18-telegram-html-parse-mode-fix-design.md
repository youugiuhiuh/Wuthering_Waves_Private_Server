# Telegram HTML ParseMode Bugfix Design

## Problem

Telegram 消息中 HTML 标签（`<b>`、`<code>` 等）被显示为字面文本，而非渲染为格式。

## Root Cause

Telegram Bot API 要求发送消息时显式设置 `parse_mode=HTML`。`TelegramAdapter`（`rust/aegis/src/adapters/telegram/adapter.rs`）的 `send_message()` 和 `edit_message()` **正确设置了** `.parse_mode(ParseMode::Html)`。

但许多 handler 文件直接调用 teloxide `Bot` 的 `send_message()` / `edit_message_text()`，**跳过 adapter，也没有设置 parse_mode**，导致 HTML 标签被当作纯文本显示。

## Affected Files & Counts

| File | send_message bugs | edit_message_text bugs | Total |
|------|-------------------|------------------------|-------|
| `handlers/message.rs` | 8 | 0 | 8 |
| `handlers/ops.rs` | 5 | 0 | 5 |
| `handlers/menu.rs` | 3 | 6 | 9 |
| `handlers/schedule.rs` | 0 | 1 | 1 |
| **Total** | **16** | **7** | **23** |

Clean files (no bugs): `log.rs`, `xray.rs`, `warp.rs`, `singbox.rs`, `context.rs`

## Approach: Direct ParseMode Addition (方案 A)

Add `.parse_mode(ParseMode::Html)` to each of the 23 missing locations.

- `Import addition`: where `ParseMode` is not yet imported, add `use teloxide::types::ParseMode;`
- `Send`: change `bot.send_message(chat_id, text).await?;` → `bot.send_message(chat_id, text).parse_mode(ParseMode::Html).await?;`
- `Edit`: change `bot.edit_message_text(chat_id, msg_id, text).await?;` → `bot.edit_message_text(chat_id, msg_id, text).parse_mode(ParseMode::Html).await?;`
- For calls that already have `.reply_markup(...)` chained, insert `.parse_mode(ParseMode::Html)` before `.reply_markup(...)`
- For calls inside `tokio::spawn` (no `?`), use `.await;` as-is

## Risk Assessment

- **Low risk**: mechanical change, no logic alteration
- **Low risk**: `ParseMode::Html` is idempotent — setting it where text has no HTML produces no change
- **Medium risk**: import collisions — verify `ParseMode` is not already imported with a different path

## Verification

- `cargo check` must pass (compilation)
- `cargo test` must pass (existing tests)
- `cargo fmt` must pass (formatting)
- Manual: confirm Telegram messages with `<b>` render correctly
