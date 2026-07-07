# Discord Integration (rust/aegis)

**Date**: 2026-07-07
**Status**: Design / Reference
**Platform**: Discord

## 1. 目標與範圍

- 支持平台：Discord
- 支持功能：基於 `BotAdapter` 的消息收發、文字命令分發、按鈕渲染
- **不支持**：自毀流程、調度管理（保留未來擴展接口）
- 交互方式：文字命令（未來可考慮 Slash Commands）

## 2. serenity 關鍵 API

Crate：`serenity`（官方倉庫：<https://github.com/serenity-rs/serenity>）

| API | 用途 | 對應 adapter 方法 |
|-----|------|-------------------|
| `Client::builder(token, GatewayIntents).await` | 建立 bot 客戶端 | 初始連接 |
| `Http::send_message(channel_id, CreateMessage)` | 發送訊息 | `send_message` |
| `ChannelId::edit_message(http, msg_id, EditMessage)` | 編輯訊息 | `edit_message` |
| `ChannelId::delete_message(http, msg_id)` | 刪除訊息 | `delete_message` |
| `CreateButton::new(custom_id).label(text).style(style)` | 建立按鈕 | `Markup` → `CreateActionRow` |
| `CreateActionRow::Buttons(buttons)` | 建立按鈕行 | `Markup.buttons` |

### 重要類型

- `ChannelId`：Discord 頻道 ID（`u64`），對應 `TargetId`
- `MessageId` (Serenity)：Discord 訊息 ID（`u64`），對應 `MessageId`
- `Http`：Discord API 的 HTTP 客戶端，`Arc<Http>` 可在多處共享

## 3. 現有 `DiscordAdapter` 解析

文件位置：`src/adapters/discord/adapter.rs`

### BotAdapter 接口實現

| 方法 | 實作方式 |
|------|----------|
| `send_message` | `ChannelId::send_message(http, CreateMessage)` + 按鈕轉 `CreateActionRow` |
| `edit_message` | `ChannelId::edit_message(http, msg_id, EditMessage)` |
| `delete_message` | `ChannelId::delete_message(http, msg_id)` |
| `platform` | 返回 `Platform::Discord` |

### Markup → Discord 按鈕轉換

`convert_markup_discord(markup: &Markup) -> Vec<CreateActionRow>`：
- 每行 `Markup.buttons` 轉為一組 `CreateActionRow::Buttons`
- 按鈕樣式固定為 `ButtonStyle::Primary`
- callback data（`btn.data`）對應我們的 `BTN_DESTROY_*` 常量

### target id 處理

- `TargetId` 存的是 Discord 的 `ChannelId`（`u64` 的字串表示）
- 每次操作需先 parse 為 `u64` 再轉 `ChannelId`

## 4. 缺失部分與建議實作

Discord 目前缺少命令解析和事件分發層，需要新建：

```
src/adapters/discord/
├── mod.rs            # 已存在，匯出 adapter
├── adapter.rs        # 已存在
├── commands.rs       # 新增：命令解析
└── handlers.rs       # 新增：事件處理與分發
```

### 建議的命令系統

採用**文字命令**（prefix `!`）：

```
!auth <code>      - TOTP 驗證
!help / !h        - 幫助訊息
!status           - 系統狀態
!menu             - 功能選單
!xray status/add/del/pq status/pq gen   - Xray 管理
!sb / !singbox status/add/del           - SingBox 管理
!ops reload/upgrade/maintenance/bbr3/geo/fw - 系統操作
!warp status/install/uninstall          - WARP 管理
!destruct         - 暫不支持
!sched list/add/del                      - 暫不支持
```

### 命令解析器

可複用 `matrix/commands.rs` 的模式：

```rust
pub fn parse(text: &str) -> Command {
    let trimmed = text.strip_prefix('!').unwrap_or(text).trim();
    // ... 同 matrix/commands.rs 邏輯
}
```

若未來升級為 Slash Commands，需在 `serenity::Client` 的事件 loop 中註冊 `poise` 或 `serenity::framework`。

## 5. 按鈕交互與 DestructInput 映射

雖然自毀流程不支持，但按鈕框架已存在：

| 操作 | `Markup.buttons` 內容 | `data` 值 |
|------|----------------------|-----------|
| 確認 | `CreateButton::new("a_destroy_confirm")` | `BTN_DESTROY_CONFIRM` |
| 取消 | `CreateButton::new("a_destroy_cancel")` | `BTN_DESTROY_CANCEL` |

若未來啟用自毀，handler 中將 callback data 傳入 `DestructInput::Button(data)` 即可。

## 6. runtime 接線方式

在 `main.rs` / `main/runtime.rs` 中新增 Discord 初始化：

```rust
use serenity::all::{GatewayIntents, EventHandler, Context};
use serenity::Client;

// 建立 HTTP 客戶端
let http = Arc::new(Http::new(&token));
let adapter = Arc::new(DiscordAdapter::new(http.clone())) as Arc<dyn BotAdapter>;

struct Handler { state: Arc<AppState>, adapter: Arc<dyn BotAdapter> }

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.id == ctx.cache.current_user_id { return; }
        // 解析文字命令 → 分發
    }
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // 若使用 Slash Commands，在此處理
    }
}

let mut client = Client::builder(&token, GatewayIntents::all())
    .event_handler(Handler { state, adapter })
    .await?;
client.start().await?;
```

## 7. 與 Telegram 的差異

| 維度 | Telegram | Discord |
|------|----------|---------|
| 消息格式 | `ChatId(-100xxx)` | `ChannelId(u64)` |
| 按鈕 | `InlineKeyboardMarkup` | `CreateActionRow::Buttons` |
| 編輯消息 | `edit_message_text(chat_id, msg_id, text)` | `edit_message(channel_id, msg_id, EditMessage)` |
| 刪除消息 | `delete_message(chat_id, msg_id)` | `delete_message(channel_id, msg_id)` |
| 回調 | `CallbackQuery` / `answer_callback_query` | `Interaction::MessageComponent` |
| 命令 | `/slash` 命令或內嵌按鈕 | `!prefix` 或 Slash Commands |
| 頻道 id 格式 | `i64` | `u64` |

## 8. 已知限制

- 自毀流程不支持（高風險操作僅限 Telegram）
- 調度管理不支持（暫保留）
- Discord 訊息長度限制（4000 chars），超長需分頁
- 文件上傳限制（25MB，無 Nitro）

## 9. TODO / 未來方向

- [ ] 建立 `discord/commands.rs`（命令解析器，參考 `matrix/commands.rs`）
- [ ] 建立 `discord/handlers.rs`（事件分發，參考 `matrix/handlers.rs`）
- [ ] 在 `main.rs` / `main/runtime.rs` 接入 Discord runtime
- [ ] 考慮評估 `poise` 框架支持 Slash Commands
- [ ] 評估建立統一命令註冊表避免 Telegram / Matrix / Discord 各自維護命令列表
