# Aegis × serenity (Discord) 完整平台集成

## 目標

將 Discord 從僅發送 stub 提升為與 Telegram/Matrix 同級的一等平台：透過 `serenity::Client` + `EventHandler` 接收事件，翻譯為 `BotEvent` → `dispatch_event()`，零業務邏輯重複。

## 架構

```
serenity::Client (gateway) ── EventHandler ──┐
  ├─ Message                → BotEvent::Message  │
  ├─ Interaction::Command   → defer → Command    → dispatch_event()
  └─ Interaction::Component → defer → Callback   │
                                                 │
DiscordAdapter(Arc<Http>) ← client.http.clone() ─┘
  send/edit/delete/download (已有，僅構造來源變)
```

Discord 獨立運行 (`--discord` 等同 `--matrix`)，不與 Telegram 通過 RoutingAdapter 組合（不同 target 語義）。

Setup CLI 新增可選參數 `--discord-token <token> --discord-admin-id <id>`；`EncryptedConfig` 儲存方式與 matrix_* 相同。

## 事件映射

| serenity 事件 | BotEvent | 字段 |
|---|---|---|
| `message` (DM) | `Message` | `target=channel_id`, `user_id=author.id.0 as i64`, `text=msg.content`, `file_id=attachments[0].url` |
| `Interaction::Command` (slash) | `Command` | 按 `data.name` 映射 `BotCommand::{Help,Start,Menu,Auth{code},SetSecurityFile}` |
| `Interaction::Component` (按鈕) | `Callback` | `data=component.data.custom_id`, `user_id=interaction.user.id.to_string()`, `msg_id=interaction.message.id` |

## 檔案異動

| 檔案 | 動作 | 內容 |
|---|---|---|
| `src/adapters/discord/adapter.rs` | 修改 | 構造來源改為 `client.http.clone()`；`capabilities()` 加 `DISCORD` const |
| `src/adapters/common/trait.rs` | 修改 | 加 `PlatformCapabilities::DISCORD` const |
| `src/main/discord.rs` | **新建** | `DiscordHandle` 元組別名、`has_discord_config()`、`connect_discord()`、`register_slash_commands()` |
| `src/main/runtime.rs` | 修改 | Discord 分支：註冊 EventHandler, `interaction.defer()` 後 dispatch, `spawn(client.start())` |
| `src/main.rs` | 修改 | CLI `--discord`, `--all` 含 discord, `connect_discord`, 傳給 runtime |
| `src/main/mod.rs` | 修改 | `pub mod discord;` |
| `src/app/state.rs` | 修改 | `is_admin_user` 加 `discord_admin_id` 判別（2 行） |
| `src/bootstrap.rs` | 修改 | `EncryptedConfig` 加 `discord_token/discord_admin_id: Option<Vec<u8>>` |
| `src/main/config.rs` | 修改 | `DecryptedConfig` 解密 `discord_token/discord_admin_id`；`validate_decrypted_config` 對 `discord_token` 僅檢查非空（跳過 Telegram `<bot_id>:<token>` 格式） |

### 不變

- **不**加 cargo feature gate（serenity 已非 optional 編譯；tg-only 體積要緊時再加）
- **不**改動 `src/main/adapter.rs`（Discord 獨立運行，不走 RoutingAdapter）
- **不**移除 poise（零引用但屬無關清理，另起 PR）
- **不**動 `shared/dispatch.rs`、`commands.rs`、`destruct.rs`（user_id 維持 i64 cast）

## DiscordAdapter 改動

構造改為 `DiscordAdapter::new(client.http.clone())`。`send/edit/delete` 已正確（`ChannelId::new(target.0.parse())` + `CreateMessage`）。`answer_callback` 維持 no-op（Discord 在 EventHandler 內 defer）。`capabilities()` 返回新 `DISCORD` const。

## main/discord.rs（新建）

```
type DiscordHandle = (serenity::Client, serenity::all::ChannelId, Arc<dyn BotAdapter>);

has_discord_config(&enc, &args) → args --discord/--all || 兩字段均存在

connect_discord(security, enc, config_dir):
  解密 discord_token/discord_admin_id
  intents = DIRECT_MESSAGES | MESSAGE_CONTENT
  Client::builder(token, intents).event_handler(DiscordHandler{...}).await
  admin_user = UserId::new(admin_id as u64)
  admin_channel = admin_user.create_dm_channel(&http).await?.id
  adapter = DiscordAdapter::new(http)
  register_slash_commands(&http, [help,start,menu,auth,setsecurityfile]).await
  (client, admin_channel, Arc::new(adapter))
```

`DiscordHandler` 持 `Arc<AppState>` + `Arc<dyn BotAdapter>` + `admin_channel`。

## runtime.rs Discord 分支

EventHandler 內：
- `message`：僅處理 `admin_channel` 內訊息 → `BotEvent::Message` → `dispatch_event`
- `interaction_create`：
  - 先 `interaction.defer(&ctx.http).await`（Discord 3s 硬限制）
  - `Component` → `BotEvent::Callback`
  - `Command` → 按 name 映射 `BotEvent::Command`
  - → `dispatch_event`
- `tokio::spawn(client.start())`

Discord-only 時 scheduler/notify 初始化同 Matrix-only 模式（CancellationToken 保活）。

## AppState 鑑權（根因，2 行）

```
fn is_admin_user(&self, user_id: i64) -> bool {
    user_id == self.admin_id
        || self.discord_admin_id.map_or(false, |d| user_id == d)
}
```
`AppState::new` 加 `discord_admin_id: Option<i64>` 參。`main.rs` 建構時傳入。

## 測試

| 範圍 | 方法 |
|---|---|
| slash name → `BotCommand` 映射 | 映射函數單測 |
| `is_admin_user` 含/不含 discord_admin_id | state.rs 單測 |
| `has_discord_config` 各組合 | main/discord.rs 單測 |
| 回歸 | 現有 `cargo test` 全綠 |

## 已知限制/運維前提

1. `MESSAGE_CONTENT`（或 `MESSAGE_CONTENT_INTENT`）為 Privileged Intent，需在 Discord Developer Portal 手動開啟，否則 DM 文字為空串（TOTP 自由輸入/destruct 文字流依賴此）。
2. bot 與 admin 須至少共處一個 guild 才能 `create_dm_channel`。
3. `admin_id`(i64) 維持 Telegram admin；`discord_admin_id: Option<i64>` 是獨立新字段。Matrix 仍復用 `admin_id`（既有行為不變）。
4. Discord token（`MTIz.NDAx.abc`）格式與 Telegram 不同，validator 跳過 `:` 格式檢查。
