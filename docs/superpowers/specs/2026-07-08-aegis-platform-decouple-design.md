# Aegis 平台通用化架构设计

**日期**: 2026-07-08
**目标**: 将业务代码从 Telegram 解耦，构建统一的多平台 Bot 架构
**策略**: 两阶段演进 — Phase A 提取共享 Handler（立即）→ Phase B 统一事件总线（后续）

---

## 1. 现状分析

### 已解耦部分（良好）

- `src/core/` — 所有业务逻辑（xray/singbox/security/system）与平台无关
- `src/adapters/common/trait.rs` — `BotAdapter` trait 抽象 send/edit/delete
- `src/adapters/{telegram,discord,matrix}/adapter.rs` — 三平台均实现 `BotAdapter`
- `src/app/state.rs` — AppState 通过 `Arc<dyn BotAdapter>` 操作，平台无关

### 需解耦部分

- `src/main.rs` — 直接使用 teloxide（`Bot`, `Message`, `ChatId`, `#[derive(BotCommands)]`, file download）
- `src/adapters/telegram/handlers/` — 9 个 Handler 全部依赖 teloxide 类型
- `src/adapters/telegram/handlers/context.rs` — `CallbackContext` 包裹 `Bot` + `CallbackQuery`
- `src/main/runtime.rs` — Telegram Dispatcher 硬编码，Matrix 独立路径
- `src/bootstrap.rs` — token 校验强制 Telegram 格式 `<bot_id>:<token>`
- `user_id` 类型为 `i64`（Telegram 专有），其他平台用 `u64`/`String`

### 平台 SDK 能力差异（影响设计）

| 能力 | Telegram | Discord | LINE | Signal (presage) |
|------|----------|---------|------|-------------------|
| 编辑消息 | Yes | Yes | No | Yes |
| 删除消息 | Yes | Yes | No | Yes |
| 内联键盘 | Yes | Yes | Yes | No |
| 斜杠命令 | `#[BotCommands]` | Slash cmd / poise | 无（手动解析） | 无（裸消息） |
| 文件收发 | Yes | Yes | Yes (image/video/file) | Yes (attachment) |
| User ID 类型 | `i64` | `u64` | String | UUID |
| 连接方式 | Long-polling | WebSocket | Webhook HTTP | WebSocket |
| 回复消息 | `reply_to_message_id` | `referenced_message` | `replyToken` | Quote message |

**关键设计约束**：Handler 内部通过 `PlatformCapabilities` 查询当前平台能力，UI 自动降级。

---

## 2. 目标架构

```
┌─────────────────────────────────────────────────┐
│  Shared Handlers (src/shared/handlers/)         │
│  menu / ops / singbox / xray / warp / log /     │
│  schedule / callback / message                  │
│                                                 │
│  所有 Handler 仅依赖：                           │
│  - &dyn BotAdapter (操作所有平台)                │
│  - CallbackEvent (无 Teloxide 类型)              │
│  - AppState                                     │
├─────────────────────────────────────────────────┤
│  Adapter Layer                                  │
│  BotAdapter trait + PlatformCapabilities        │
│  TelegramAdapter / DiscordAdapter / MatrixAdapter│
│  未来: LineAdapter / SignalAdapter              │
├─────────────────────────────────────────────────┤
│  Event Sources (Phase B)                        │
│  EventSource trait + HandlerEvent enum          │
│  TelegramEventSource / DiscordEventSource / ... │
└─────────────────────────────────────────────────┘
```

---

## 3. Phase A — 共享 Handler 提取（立即执行）

### 3.1 新增类型

```rust
// src/shared/types.rs

/// 平台无关的回调事件（替代 CallbackContext）
pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,           // ← 从 i64 改为 String
    pub msg_id: MessageId,
    pub data: String,
}
```

### 3.2 BotAdapter trait 扩展

```rust
// src/adapters/common/trait.rs

#[async_trait]
pub trait BotAdapter: Send + Sync {
    // ——— 现有方法 ———
    fn platform(&self) -> Platform;
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;
    async fn edit_message(&self, target: &TargetId, msg_id: &MessageId, content: MessageContent) -> Result<()>;
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;

    // ——— Phase A 新增 ———
    /// 应答回调（Telegram: answer_callback_query, Discord: defer interaction）
    async fn answer_callback(&self, target: &TargetId, callback_id: &str, text: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// 下载文件（仅 Telegram/Signal 支持，其他返回错误）
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        anyhow::bail!("platform does not support file download")
    }

    /// 平台能力查询
    fn capabilities(&self) -> PlatformCapabilities;
}

/// 平台能力描述
pub struct PlatformCapabilities {
    pub can_edit_message: bool,
    pub can_delete_message: bool,
    pub has_inline_keyboard: bool,
    pub has_slash_commands: bool,
    pub has_file_transfer: bool,
}

impl PlatformCapabilities {
    pub const TELEGRAM: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: true,
    };
    pub const DISCORD: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: false,
    };
    pub const MATRIX: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: false,
        has_slash_commands: false,
        has_file_transfer: true,
    };
}
```

### 3.3 文件迁移清单

| 原路径 | 新路径 | 变更内容 |
|--------|--------|---------|
| `src/adapters/telegram/handlers/mod.rs` | `src/shared/handlers/mod.rs` | dispatch 路由表，纯字符串匹配 |
| `src/adapters/telegram/handlers/context.rs` | (删除) `src/shared/types.rs` | CallbackContext → CallbackEvent |
| `src/adapters/telegram/handlers/menu.rs` | `src/shared/handlers/menu.rs` | 所有 `Bot`/`ChatId` 替换为 `&dyn BotAdapter`/`TargetId` |
| `src/adapters/telegram/handlers/singbox.rs` | `src/shared/handlers/singbox.rs` | 同上 |
| `src/adapters/telegram/handlers/xray.rs` | `src/shared/handlers/xray.rs` | 同上 |
| `src/adapters/telegram/handlers/warp.rs` | `src/shared/handlers/warp.rs` | 同上 |
| `src/adapters/telegram/handlers/ops.rs` | `src/shared/handlers/ops.rs` | 同上 |
| `src/adapters/telegram/handlers/log.rs` | `src/shared/handlers/log.rs` | 同上 |
| `src/adapters/telegram/handlers/schedule.rs` | `src/shared/handlers/schedule.rs` | 同上 |
| `src/adapters/telegram/handlers/callback.rs` | `src/shared/handlers/callback.rs` | 同上 |
| `src/adapters/telegram/handlers/message.rs` | `src/shared/handlers/message.rs` | 同上 |

### 3.4 每个 Handler 文件的改造模式（以 menu.rs 为例）

**改造前**（Telegram 专用）：
```rust
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("Monitor", "m_mon")],
    ]);
    bot.send_message(chat_id, "Main Menu")
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
}
```

**改造后**（平台无关）：
```rust
use aegis::adapters::common::{BotAdapter, Markup, InlineButton, MessageContent, TargetId};

pub async fn send_main_menu(
    adapter: &dyn BotAdapter,
    target: &TargetId,
) -> anyhow::Result<()> {
    let markup = Some(Markup {
        buttons: vec![
            vec![InlineButton { text: "Monitor".into(), data: "m_mon".into() }],
        ],
    });
    adapter.send_message(target, MessageContent {
        text: "Main Menu".into(),
        markup,
    }).await?;
    Ok(())
}
```

平台适配器负责将 `Markup`/`InlineButton` 翻译为各平台原生类型（已在 `TelegramAdapter::send_message` 中实现 `convert_markup`，同样逻辑在 `DiscordAdapter`/`MatrixAdapter` 中）。

### 3.5 Telegram 兼容层

`src/adapters/telegram/handlers/` 目录保留回调入口文件，负责 Teloxide → CallbackEvent 的翻译：

```rust
// src/adapters/telegram/dispatch.rs
async fn handle_command(bot: Bot, msg: Message, cmd: Command, state: Arc<AppState>) -> ResponseResult<()> {
    let target = TargetId(msg.chat.id.0.to_string());
    let user_id = msg.from.as_ref().map(|u| (u.id.0 as i64).to_string());
    // 翻译 → 调用 shared handler
    shared::handlers::menu::send_main_menu(&*state.adapter, &target).await?;
}
```

确保 Telegram 在 Phase A 期间 100% 兼容，无功能降级。

---

## 4. Phase B — 统一事件总线（Phase A 完成后执行）

### 4.1 核心概念

每个平台通过实现 `EventSource` trait 接入统一路由器。新平台只需两个步骤：
1. 实现 `BotAdapter`（适配 send/edit/delete + 能力检测）
2. 实现 `EventSource`（将原生事件翻译为 `HandlerEvent`，调用 `router.dispatch()`）

### 4.2 统一事件类型

```rust
// src/shared/event.rs

pub enum HandlerEvent {
    Message {
        user_id: String,
        target: TargetId,
        text: String,
    },
    Callback {
        user_id: String,
        target: TargetId,
        msg_id: MessageId,
        data: String,
    },
    Attachment {
        user_id: String,
        target: TargetId,
        file_data: Vec<u8>,
        mime_type: String,
        file_name: String,
    },
}
```

### 4.3 EventSource trait

```rust
// src/sources/mod.rs

#[async_trait]
pub trait EventSource: Send + Sync + 'static {
    fn platform(&self) -> Platform;
    async fn run(
        self: Arc<Self>,
        adapter: Arc<dyn BotAdapter>,
        router: Arc<SharedMessageRouter>,
    ) -> Result<()>;
}
```

### 4.4 SharedMessageRouter

```rust
// src/shared/router.rs

pub struct SharedMessageRouter {
    adapter: Arc<dyn BotAdapter>,
    state: Arc<AppState>,
}

impl SharedMessageRouter {
    pub async fn dispatch(&self, event: HandlerEvent) -> Result<()> {
        match event {
            HandlerEvent::Message { user_id, target, text } => {
                if is_totp_code(&text) {
                    return auth::process(&self.state, &target, &user_id, &text).await;
                }
                if let Some(cmd) = parse_command(&text) {
                    return self.handle_command(cmd, &target, &user_id).await;
                }
                message_handler::handle(&self.state, &*self.adapter, &target, &user_id, &text).await
            }
            HandlerEvent::Callback { user_id, target, msg_id, data } => {
                callback_handler::dispatch(&self.state, &*self.adapter, &target, &msg_id, &user_id, &data).await
            }
            HandlerEvent::Attachment { user_id, target, file_data, mime_type, file_name } => {
                attachment_handler::handle(&self.state, &*self.adapter, &target, &user_id, &file_data, &mime_type, &file_name).await
            }
        }
    }
}
```

### 4.5 各平台 EventSource 实现要点

| 平台 | EventSource::run() 实现 | 原生事件 → HandlerEvent 映射 |
|------|------------------------|------------------------------|
| Telegram | `teloxide::Dispatcher::dispatch()` 阻塞 | `Update::Message` → `HandlerEvent::Message` |
| Discord | `serenity::Client::start()` 阻塞 | `Interaction::Component` → `HandlerEvent::Callback` |
| Matrix | `matrix_sdk::sync()` stream | `SyncRoomMessageEvent` → `HandlerEvent::Message` |
| LINE (未来) | 内嵌 `axum` HTTP server 监听 webhook | `PostbackEvent` → `HandlerEvent::Callback` |
| Signal (未来) | `presage::Manager::receive_messages()` stream | `Content::DataMessage` → `HandlerEvent::Message` |

### 4.6 Phase B 目录结构（最终形态）

```
src/
├── main.rs              # 初始化 → 启动所有 EventSource
├── bootstrap.rs         # (不变)
├── lib.rs               # 导出 core + adapters + shared
├── app/                 # (不变)
├── core/                # (不变)
├── shared/
│   ├── mod.rs
│   ├── types.rs         # CallbackEvent
│   ├── event.rs         # HandlerEvent enum
│   ├── router.rs        # SharedMessageRouter
│   └── handlers/        # 共享业务 Handler
├── sources/             # Phase B 新增
│   ├── mod.rs           # EventSource trait
│   ├── telegram.rs
│   ├── discord.rs
│   └── matrix.rs
├── adapters/
│   ├── common/
│   │   ├── mod.rs
│   │   ├── trait.rs     # BotAdapter + PlatformCapabilities
│   │   └── routing.rs
│   ├── telegram/adapter.rs
│   ├── telegram/dispatch.rs
│   ├── discord/adapter.rs
│   └── matrix/adapter.rs
└── main/
    ├── config.rs
    └── cli.rs
```

---

## 5. 迁移策略

```
Phase A（立即执行）
├── Step 1: 创建 src/shared/ + types.rs + handlers/mod.rs（空目录结构）
├── Step 2: 扩展 BotAdapter trait（answer_callback + capabilities + download_file）
├── Step 3: 改造第一个 Handler（menu.rs）验证模式可行
├── Step 4: 逐个迁移剩余 8 个 Handler
│   每次迁移：旧 Handler 转为兼容层 → 测试 Telegram 正常 → 下一个
├── Step 5: 简化 main.rs（命令路由通过 shared handler）
├── Step 6: Telegram 全功能回归测试通过

Phase B（Phase A 完成后）
├── Step 7: 创建 src/sources/ + EventSource trait
├── Step 8: Telegram 迁移到 TelegramEventSource
├── Step 9: 实现 DiscordEventSource（添加 gateway + EventHandler）
├── Step 10: 简化 main.rs 为多平台启动入口
```

### 约束条件

- 每个 Step 完成后 `cargo test` 必须全绿
- Telegram 功能在整个迁移过程中不发生降级
- 每个 Handler 文件改造后立即验证

---

## 6. 测试策略

| 测试层次 | 内容 | 方法 |
|---------|------|------|
| 单元测试 | 每个 Handler 函数用 `MockBotAdapter` 测试 | 已有模式，继续使用 `mockall` |
| 集成测试 | CallbackEvent 分发 → Handler → Adapter 全链路 | `tests/integration_security.rs` 风格 |
| 平台回归 | Telegram 端到端（命令/回调/文件） | 保持现有 `tests/cli_*.rs` + manual test |
| Phase B 新增 | EventSource → Router → Handler 管线 | `tests/integration_router.rs` |

---

## 7. 风险与缓解

| 风险 | 缓解 |
|------|------|
| Handler 改造引入回归 | 每个文件改造后立即跑 `cargo test --lib` + `cargo test --test integration_*` |
| Markup 抽象不足以支持复杂 layout | `Markup` 已有 `Vec<Vec<InlineButton>>`，覆盖 2D 键盘；必要时加 `max_buttons_per_row: Option<usize>` |
| Signal 无 inline keyboard | Handler 通过 `capabilities().has_inline_keyboard` 检测，无键盘时用纯文本选项列表 fallback |
| LINE 不支持消息编辑 | Handler 通过 `capabilities().can_edit_message` 检测，禁止 edit 操作的 UI 隐藏 edit 相关按钮 |
