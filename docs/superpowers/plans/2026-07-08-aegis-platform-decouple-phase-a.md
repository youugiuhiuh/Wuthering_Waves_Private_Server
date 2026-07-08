# Aegis Platform Decouple (Phase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract all business logic from `src/adapters/telegram/handlers/` into a shared `src/shared/handlers/` layer that operates through `&dyn BotAdapter` + common types, with zero functionality change for Telegram.

**Architecture:** Each handler file is migrated one-by-one. The old Telegram handler files become thin compat wrappers that translate Teloxide types (`Bot`, `ChatId`, `CallbackQuery`) into shared types (`&dyn BotAdapter`, `TargetId`, `CallbackEvent`), then call the shared handler. Telegram's Dispatcher remains unchanged.

**Tech Stack:** Rust + tokio + teloxide 0.13 + serenity 0.12 + matrix-sdk 0.18

## Global Constraints

- `unsafe_code = "allow"` in Cargo.toml (already set, do not touch)
- `#[allow(clippy::vec_init_then_push)]` in main.rs (keep)
- All existing tests must pass after each task
- Telegram functionality must NOT regress at any point
- Do NOT change Cargo.toml dependencies unless absolutely necessary
- Use `mockall` for MockBotAdapter (already a dependency)
- Follow existing code style: Chinese comments on business logic, English on types
- `user_id` type changes from `i64` to `String` in shared handlers only; Telegram compat layer converts back/forth

## File Structure

```
NEW: src/shared/
  mod.rs              — module declarations
  types.rs            — CallbackEvent, HandlerAction, HandlerResult
  handlers/
    mod.rs            — dispatch() function, re-exports
    menu.rs           — menu building (from telegram/handlers/menu.rs)
    singbox.rs        — singbox management (from telegram/handlers/singbox.rs)
    xray.rs           — xray management (from telegram/handlers/xray/...)
    warp.rs           — warp routing (from telegram/handlers/warp.rs)
    ops.rs            — operations (from telegram/handlers/ops/...)
    log.rs            — log viewing (from telegram/handlers/log.rs)
    schedule.rs       — schedule (from telegram/handlers/schedule/...)
    message.rs        — message handling (from telegram/handlers/message.rs)

MODIFIED:
  src/lib.rs                      — add `pub mod shared;`
  src/adapters/common/trait.rs    — add PlatformCapabilities, answer_callback, download_file
  src/adapters/telegram/adapter.rs — implement PlatformCapabilities
  src/adapters/discord/adapter.rs  — implement PlatformCapabilities
  src/adapters/matrix/adapter.rs   — implement PlatformCapabilities
  src/adapters/telegram/handlers/callback.rs — simplify to compat wrapper
  src/adapters/telegram/handlers/message.rs — simplify to compat wrapper
  src/adapters/telegram/handlers/mod.rs — simplify to delegate to shared dispatch
  src/adapters/telegram/handlers/* — each becomes a thin compat wrapper
  src/main.rs                     — simplify command routing, notify functions

REMOVED:
  src/adapters/telegram/handlers/context.rs — replaced by shared/types.rs::CallbackEvent
```

---

### Task 1: Create shared infrastructure + extend BotAdapter

**Files:**
- Create: `src/shared/mod.rs`
- Create: `src/shared/types.rs`
- Modify: `src/adapters/common/trait.rs`
- Modify: `src/adapters/telegram/adapter.rs`
- Modify: `src/adapters/discord/adapter.rs`
- Modify: `src/adapters/matrix/adapter.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `shared::types::CallbackEvent`, `shared::types::HandlerAction`, `shared::types::HandlerResult`
- Produces: `BotAdapter::answer_callback()`, `BotAdapter::download_file()`, `BotAdapter::capabilities()`
- Produces: `PlatformCapabilities` struct + `TELEGRAM/DISCORD/MATRIX` constants

- [ ] **Step 1.1: Add PlatformCapabilities to trait.rs**

```rust
// src/adapters/common/trait.rs — add after InlineButton

#[derive(Debug, Clone, Copy)]
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
}
```

- [ ] **Step 1.2: Add new methods to BotAdapter trait**

In `trait.rs`, add these methods to the trait with default implementations:

```rust
#[async_trait]
pub trait BotAdapter: Send + Sync {
    // ... existing methods ...

    async fn answer_callback(&self, _target: &TargetId, _callback_id: &str, _text: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
        anyhow::bail!("platform does not support file download")
    }

    fn capabilities(&self) -> PlatformCapabilities;
}
```

- [ ] **Step 1.3: Add mock methods to mockall**

In `trait.rs`, the `#[mockall::automock]` attribute is on the trait. Add to the mock config:

```rust
#[mockall::automock]
#[async_trait]
pub trait BotAdapter: Send + Sync {
    // ...
    async fn answer_callback(&self, target: &TargetId, callback_id: &str, text: Option<&str>) -> Result<()>;
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>>;
    fn capabilities(&self) -> PlatformCapabilities;
}
```

Note: mockall derives `Expectation` for the new methods automatically.

- [ ] **Step 1.4: Implement capabilities in TelegramAdapter**

```rust
// src/adapters/telegram/adapter.rs — add method to impl BotAdapter for TelegramAdapter

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities::TELEGRAM
}
```

Also add `answer_callback` implementation for Telegram:

```rust
async fn answer_callback(&self, _target: &TargetId, callback_id: &str, text: Option<&str>) -> Result<()> {
    let mut answer = self.bot.answer_callback_query(callback_id);
    if let Some(t) = text {
        answer = answer.text(t);
    }
    answer.await?;
    Ok(())
}
```

- [ ] **Step 1.5: Implement capabilities in DiscordAdapter**

```rust
// src/adapters/discord/adapter.rs

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: false,
    }
}
```

- [ ] **Step 1.6: Implement capabilities in MatrixAdapter**

```rust
// src/adapters/matrix/adapter.rs

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: false,
        has_slash_commands: false,
        has_file_transfer: true,
    }
}
```

- [ ] **Step 1.7: Create src/shared/mod.rs**

```rust
pub(crate) mod types;
pub(crate) mod handlers;
```

- [ ] **Step 1.8: Create src/shared/types.rs**

```rust
use std::sync::Arc;
use crate::adapters::common::{BotAdapter, MessageId, TargetId};
use anyhow::Result;

pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,
    pub msg_id: MessageId,
    pub data: String,
    pub callback_id: String,
}

pub enum HandlerAction {
    Done,
    Redirect(String),
}

pub type HandlerResult = Result<HandlerAction>;
```

- [ ] **Step 1.9: Update lib.rs**

```rust
pub mod adapters;
pub mod core;
pub(crate) mod shared;  // match visibility in existing codebase
```

- [ ] **Step 1.10: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

Expected: all existing tests pass.

- [ ] **Step 1.11: Commit**

```bash
git add -A && git commit -m "feat(aegis): add shared infrastructure and extend BotAdapter trait"
```

---

### Task 2: Migrate dispatch table + callback.rs

**Files:**
- Create: `src/shared/handlers/mod.rs`
- Modify: `src/adapters/telegram/handlers/mod.rs`
- Create: `src/shared/handlers/callback.rs`
- Modify: `src/adapters/telegram/handlers/callback.rs`

**Interfaces:**
- Consumes: `shared::types::*`, `adapters::common::*`
- Produces: `shared::handlers::dispatch(event)` (not used by Telegram until Task 12)

**IMPORTANT:** The Telegram callback.rs keeps using its existing dispatch (the old `handlers::dispatch` in `telegram/handlers/mod.rs`) until ALL handlers are migrated to shared. This prevents regression. The shared dispatch is created in parallel and only swapped in Task 12.

- [ ] **Step 2.1: Create shared/handlers/mod.rs**

The dispatch function handles callback data routing. Create it as a pure string-matching dispatch (not wired into Telegram flow yet):

```rust
pub(crate) mod callback;
pub(crate) mod log;
pub(crate) mod menu;
pub(crate) mod message;
pub(crate) mod ops;
pub(crate) mod schedule;
pub(crate) mod singbox;
pub(crate) mod warp;

use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};

pub async fn dispatch(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();

    if data == "m_log" || data.starts_with("l_") {
        return Ok(Some(log::handle(event).await?));
    }
    if data == "m_singbox_mgmt" || data == "sb_install" || data.starts_with("sb_") {
        return Ok(Some(singbox::handle(event).await?));
    }
    if data == "m_warp" || data == "a_inst_warp" || data.starts_with("a_warp_") {
        return Ok(Some(warp::handle(event).await?));
    }
    if data == "m_sched" || data == "a_geo_sched_menu" || data == "geo_sched_off" || data.starts_with("s_") {
        return Ok(Some(schedule::handle(event).await?));
    }
    if data.starts_with("a_bbr3") || data == "a_fw" || data == "a_one_click" || data == "a_reload"
        || data == "a_sys_maint" || data == "a_sys_reboot" || data == "a_upgrade"
        || data == "a_geo" || data == "a_tune"
    {
        return Ok(Some(ops::handle(event).await?));
    }
    if data == "m_xray_mgmt" || data == "m_routing" || data.starts_with("routing_toggle:")
        || data == "m_del_cfg" || data == "m_pq_mgmt" || data == "a_inst_base"
        || data.starts_with("u_") || data.starts_with("cfg_") || data.starts_with("m_pq_")
    {
        return Ok(Some(callback::handle(event).await?));
    }
    if matches!(data,
        "m_main" | "m_ops_center" | "m_settings" | "m_net_opt" | "m_security"
        | "m_sys_cmd" | "m_mon" | "m_usr" | "m_danger" | "m_session_timeout"
        | "a_wwps_core_menu" | "a_wwps_box_menu" | "a_wwps_box_restart"
        | "a_wwps_box_status" | "a_wwps_core_latest" | "a_wwps_core_tags"
        | "a_geo_menu"
    ) || data.starts_with("set_timeout:") || data.starts_with("wwps_core_tag:")
    {
        return Ok(Some(menu::handle(event).await?));
    }

    Ok(None)
}
```

- [ ] **Step 2.2: Keep telegram/handlers/mod.rs as-is**

Do NOT change the Telegram flow yet. The existing `dispatch` function still uses `CallbackContext` and calls the old handler files. The shared dispatch is created in parallel but not wired into Telegram until Task 12.

- [ ] **Step 2.3: Create shared/handlers/callback.rs**

This handles the xray callback routing. For now, create it as a stub that delegates to the actual xray handler (to be migrated in a later task):

```rust
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    // Task 9 will populate this with xray sub-handler dispatch
    Ok(HandlerAction::Done)
}
```

*(Telegram callback.rs stays unchanged until Task 12)*

- [ ] **Step 2.3: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

- [ ] **Step 2.4: Commit**

```bash
git add -A && git commit -m "feat(aegis): create shared dispatch structure"
```

---

*(context.rs stays until Task 12 — still needed by Telegram compat wrappers)*

---

### Task 4: Migrate menu.rs (prove pattern)

**Files:**
- Create: `src/shared/handlers/menu.rs`
- Modify: `src/adapters/telegram/handlers/menu.rs` → compat wrapper
- Modify: `src/main.rs` → update call sites

**Interfaces:**
- Consumes: `&dyn BotAdapter`, `&TargetId`, `&CallbackEvent`
- Produces: `shared::handlers::menu::send_main_menu(adapter, target)`, `shared::handlers::menu::handle(event)`

- [ ] **Step 4.1: Create shared/handlers/menu.rs header + types**

```rust
use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, MessageId, TargetId};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::bootstrap::{BOT_VERSION, BotSettings, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::utils::format_duration_human;
use aegis::core::paths::{singbox, xray};
use aegis::core::singbox::SingBoxInstaller;
use aegis::core::system::SystemMonitor;
use aegis::core::system::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use anyhow::Result;
use rust_i18n::t;
use std::path::Path;
use std::sync::Arc;
```

- [ ] **Step 4.2: Port send_main_menu to shared**

Replace:
```rust
pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> { ... }
```

With:
```rust
pub async fn send_main_menu(adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
    let mut rows = vec![
        vec![
            InlineButton { text: t!("menu.monitor").into(), data: "m_mon".into() },
            InlineButton { text: t!("menu.users").into(), data: "m_usr".into() },
        ],
        vec![InlineButton { text: t!("menu.ops").into(), data: "m_ops_center".into() }],
        vec![InlineButton { text: t!("menu.settings").into(), data: "m_settings".into() }],
    ];
    rows.push(vec![InlineButton { text: t!("menu.one_click_deploy").into(), data: "a_one_click".into() }]);
    if !aegis::core::i18n::is_lang_configured() {
        rows.push(vec![
            InlineButton { text: t!("lang.zh").into(), data: "lang:zh".into() },
            InlineButton { text: t!("lang.en").into(), data: "lang:en".into() },
            InlineButton { text: t!("lang.ja").into(), data: "lang:ja".into() },
        ]);
    }
    adapter.send_message(target, MessageContent {
        text: format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
        markup: Some(Markup { buttons: rows }),
    }).await?;
    Ok(())
}
```

- [ ] **Step 4.3: Port handle function similarly**

Replace all `InlineKeyboardButton::callback(...)` patterns with `InlineButton { text: ..., data: ... }`.
Replace all `InlineKeyboardMarkup::new(...)` with `Markup { buttons: ... }`.
Replace all `bot.edit_message_text(chat_id, msg_id, ...)` with `adapter.edit_message(target, msg_id, ...)`.
Replace all `ParseMode::Html` references (remove — Markup is format-agnostic).

Key conversion pattern:
```rust
// Before (Telegram):
let keyboard = InlineKeyboardMarkup::new(vec![
    vec![InlineKeyboardButton::callback(t!("menu.monitor"), "m_mon")],
]);
bot.edit_message_text(chat_id, msg_id, text)
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;

// After (shared):
let markup = Markup {
    buttons: vec![
        vec![InlineButton { text: t!("menu.monitor").into(), data: "m_mon".into() }],
    ],
};
adapter.edit_message(target, msg_id, MessageContent {
    text: text.into_owned(),
    markup: Some(markup),
}).await?;
```

- [ ] **Step 4.4: Update Telegram menu.rs `handle` to delegate to shared**

The Telegram dispatch (`telegram/handlers/mod.rs::dispatch`) calls `menu::handle(ctx)` where `ctx: &CallbackContext`. Keep this working by making `handle` translate and delegate:

```rust
// src/adapters/telegram/handlers/menu.rs — add at end
use crate::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::shared::types::CallbackEvent;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let event = CallbackEvent {
        adapter: ctx.state.adapter.clone(),
        target: crate::adapters::common::TargetId(ctx.chat_id.0.to_string()),
        user_id: ctx.user_id.to_string(),
        msg_id: crate::adapters::common::MessageId(ctx.msg_id.0.to_string()),
        data: ctx.data.clone(),
        callback_id: String::new(),
    };
    crate::shared::handlers::menu::handle(&event).await
}
```

- [ ] **Step 4.5: Update main.rs call sites for menu**

`send_main_menu` is called from `main.rs` line 217. Change it to call shared directly:

```rust
// In handle_command, Command::Menu arm:
Command::Menu => {
    if !state.is_authorized(user_id).await {
        bot.send_message(msg.chat.id, rust_i18n::t!("auth.required")).await?;
        return Ok(());
    }
    let target = TargetId(msg.chat.id.0.to_string());
    crate::shared::handlers::menu::send_main_menu(&*state.adapter, &target)
        .await
        .map_err(|e| teloxide::RequestError::ApiError::<teloxide::ApiError>(e.to_string()))?;
}
```

Keep `pub mod menu;` in `telegram/handlers/mod.rs` — it's still used by the old dispatch path until Task 12.

- [ ] **Step 4.6: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

- [ ] **Step 4.7: Commit**

```bash
git add -A && git commit -m "feat(aegis): migrate menu handler to shared layer"
```

---

### Task 5: Migrate message.rs

**Files:**
- Create: `src/shared/handlers/message.rs`
- Modify: `src/adapters/telegram/handlers/message.rs`

- [ ] **Step 5.1: Create shared/handlers/message.rs**

The `handle_message` function currently takes `(bot: Bot, msg: Message, state: Arc<AppState>)` and returns `ResponseResult<()>`.

Shared version takes `(adapter: &dyn BotAdapter, state: &AppState, target: &TargetId, user_id: &str, text: Option<&str>, has_document: bool, has_photo: bool)` and returns `Result<()>`.

Key changes:
- Replace `bot.send_message(chat_id, ...)` with `adapter.send_message(target, ...)`
- Replace `msg.text()` with `text` parameter
- Replace `msg.document().is_some()` / `msg.photo().is_some()` with `has_document` / `has_photo`
- Replace `ParseMode::Html` (remove, format-agnostic)
- Return `Result<()>` instead of `ResponseResult<()>`

- [ ] **Step 5.2: Update Telegram message.rs to delegate**

```rust
// Simplified compat wrapper
pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let target = crate::adapters::common::TargetId(msg.chat.id.0.to_string());
    let user_id = msg.from.as_ref().map(|u| u.id.0.to_string()).unwrap_or_default();
    crate::shared::handlers::message::handle(
        &*state.adapter,
        &state,
        &target,
        &user_id,
        msg.text(),
        msg.document().is_some(),
        msg.photo().is_some(),
    ).await.map_err(|e| teloxide::RequestError::ApiError::<teloxide::ApiError>(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 5.3: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

- [ ] **Step 5.4: Commit**

```bash
git add -A && git commit -m "feat(aegis): migrate message handler to shared layer"
```

---

### Task 6: Migrate log.rs

**Files:**
- Create: `src/shared/handlers/log.rs`
- Modify: `src/adapters/telegram/handlers/log.rs`

- [ ] **Step 6.1: Create shared/handlers/log.rs (116 lines)**

Pattern: same as menu — replace all `Bot` → `&dyn BotAdapter`, `ChatId` → `&TargetId`, `InlineKeyboardButton` → `InlineButton`, `InlineKeyboardMarkup` → `Markup`, `ParseMode::Html` → remove.

- [ ] **Step 6.2: Update Telegram log.rs to delegate**

- [ ] **Step 6.3: Run tests then commit**

---

### Task 7: Migrate ops.rs (sub-modules: mod, deploy, system, bbr3, geo, reboot, reload, sys_maint, upgrade, firewall)

**Files:**
- Create: `src/shared/handlers/ops/` directory structure (flat file: `ops.rs`)
- Modify: `src/adapters/telegram/handlers/ops/` files as compat wrappers

**Note:** The Telegram ops handlers are split across 9 files totaling 798 lines. For the shared version, merge all into a single `src/shared/handlers/ops.rs` for simplicity. The Telegram compat layer recreates the sub-module structure if needed.

- [ ] **Step 7.1: Create shared/handlers/ops.rs**

Contains all ops handler functions: `handle`, `handle_deploy`, `handle_system`, `handle_bbr3`, `handle_geo`, `handle_reboot`, `handle_reload`, `handle_sys_maint`, `handle_upgrade`, `handle_firewall`.

Each function uses `(&dyn BotAdapter, &TargetId, ...)` instead of `(&CallbackContext)`.

- [ ] **Step 7.2: Update Telegram ops/ to delegate**

- [ ] **Step 7.3: Run tests**

---

### Task 8: Migrate singbox.rs

**Files:**
- Create: `src/shared/handlers/singbox.rs`
- Modify: `src/adapters/telegram/handlers/singbox.rs`

- [ ] **Step 8.1: Create shared/handlers/singbox.rs (676 lines)**

- [ ] **Step 8.2: Update Telegram singbox.rs to delegate**

- [ ] **Step 8.3: Run tests**

---

### Task 9: Migrate xray handlers (960 lines across 7 files)

**Files:**
- Create: `src/shared/handlers/xray.rs` (single file, merge all xray sub-modules)
- Modify: `src/adapters/telegram/handlers/xray/` files as compat wrappers

**Sub-modules to merge:**
- `xray/mod.rs` (dispatch)
- `xray/mgmt.rs`
- `xray/routing.rs`
- `xray/batch.rs`
- `xray/delete.rs`
- `xray/delete_select.rs`
- `xray/delete_count.rs`

- [ ] **Step 9.1: Create shared/handlers/xray.rs**

Merge all xray sub-handler functions into one file.

- [ ] **Step 9.2: Update Telegram xray/ to delegate**

- [ ] **Step 9.3: Run tests**

---

### Task 10: Migrate warp.rs

**Files:**
- Create: `src/shared/handlers/warp.rs`
- Modify: `src/adapters/telegram/handlers/warp.rs`

- [ ] **Step 10.1: Create shared/handlers/warp.rs (425 lines)**

- [ ] **Step 10.2: Update Telegram warp.rs to delegate**

- [ ] **Step 10.3: Run tests**

---

### Task 11: Migrate schedule handlers (1048 lines across 3 files)

**Files:**
- Create: `src/shared/handlers/schedule.rs` (single file)
- Modify: `src/adapters/telegram/handlers/schedule/` files as compat wrappers

- [ ] **Step 11.1: Create shared/handlers/schedule.rs**

- [ ] **Step 11.2: Update Telegram schedule/ to delegate**

- [ ] **Step 11.3: Run tests**

---

### Task 12: Wire shared dispatch into Telegram + Simplify main.rs

**Files:**
- Modify: `src/main.rs`
- Modify: `src/adapters/telegram/handlers/callback.rs`
- Modify: `src/adapters/telegram/handlers/mod.rs`

**This is the final integration step:** all handlers are now migrated to shared, so Telegram can safely switch to the shared dispatch.

- [ ] **Step 12.1: Wire shared dispatch into telegram/handlers/mod.rs**

Replace the mod.rs dispatch to delegate to shared:

```rust
pub(crate) mod callback;
pub mod context;
pub(crate) mod message;

pub(crate) async fn dispatch(ctx: &context::CallbackContext) -> Result<Option<HandlerAction>> {
    let event = crate::shared::types::CallbackEvent {
        adapter: ctx.state.adapter.clone(),
        target: crate::adapters::common::TargetId(ctx.chat_id.0.to_string()),
        user_id: ctx.user_id.to_string(),
        msg_id: crate::adapters::common::MessageId(ctx.msg_id.0.to_string()),
        data: ctx.data.clone(),
        callback_id: String::new(),
    };
    crate::shared::handlers::dispatch(&event).await
}
```

Remove `pub mod log; pub mod ops; pub mod menu; pub mod schedule; pub mod singbox; pub mod warp;` — no longer needed as direct modules.

- [ ] **Step 12.2: Wire shared dispatch into telegram/handlers/callback.rs**

Replace the existing `handle_callback` function's loop body. Instead of calling `handlers::dispatch(&ctx)`, create a `CallbackEvent` and call `shared::handlers::dispatch(&event)`:

```rust
// Inside the loop, replace:
if let Some(action) = handlers::dispatch(&ctx).await? {
// With:
let event = crate::shared::types::CallbackEvent {
    adapter: state.adapter.clone(),
    target: crate::adapters::common::TargetId(chat_id.0.to_string()),
    user_id: user_id.to_string(),
    msg_id: crate::adapters::common::MessageId(msg_id.0.to_string()),
    data: data.clone(),
    callback_id: q.id.clone(),
};
if let Some(action) = crate::shared::handlers::dispatch(&event).await
    .map_err(|e| teloxide::RequestError::ApiError::<teloxide::ApiError>(e.to_string()))?
{
    // ... same HandlerAction::Done / HandlerAction::Redirect match ...
}
```

- [ ] **Step 12.3: Move notify functions to shared handlers**

- [ ] **Step 12.2: Simplify Command handling**

Change `handle_command` to translate Teloxide `Command` enum data into shared handler calls:

```rust
async fn handle_command(bot: Bot, msg: Message, cmd: Command, state: Arc<AppState>) -> ResponseResult<()> {
    let Some(from) = msg.from.as_ref() else { ... };
    let target = TargetId(msg.chat.id.0.to_string());
    let user_id = from.id.0 as i64;

    match cmd {
        Command::Help | Command::Start => {
            state.adapter.send_message(&target, MessageContent {
                text: rust_i18n::t!("welcome.title").into(),
                markup: None,
            }).await?;
        }
        Command::Auth(code) => {
            let _ = process_auth_code(&state, &target, user_id, &code).await;
        }
        Command::SetSecurityFile => {
            // file download logic stays Telegram-specific (uses bot.download_file)
            // but can be refactored to use adapter.download_file() when available
        }
        Command::Menu => {
            if !state.is_authorized(user_id).await {
                bot.send_message(msg.chat.id, rust_i18n::t!("auth.required"))
                    .await?;
                return Ok(());
            }
            shared::handlers::menu::send_main_menu(&*state.adapter, &target).await
                .map_err(|e| teloxide::RequestError::ApiError::<teloxide::ApiError>(e.to_string()))?;
        }
    }
    Ok(())
}
```

- [ ] **Step 12.3: Remove unused handler imports from main.rs**

Remove `mod handlers;` (no longer needed — delegate via shared) and clean up `#[path = "adapters/telegram/handlers/mod.rs"]`.

- [ ] **Step 12.4: Remove context.rs**

With the old dispatch path gone, `CallbackContext` is no longer needed. Delete `src/adapters/telegram/handlers/context.rs` and remove `pub mod context;` from `telegram/handlers/mod.rs`.

- [ ] **Step 12.5: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

- [ ] **Step 12.6: Commit**

---

### Task 13: Full test pass + final cleanup

**Files:**
- Verify: All handler files, main.rs, lib.rs

- [ ] **Step 13.1: Remove dead code**

```bash
cargo clippy 2>&1 | grep "warning: unused"
```

Fix any unused import/variable warnings.

- [ ] **Step 13.2: Full test suite**

```bash
cd rust/aegis && cargo test 2>&1
```

Expected: all tests pass, no failures.

- [ ] **Step 13.3: Commit**

```bash
git add -A && git commit -m "feat(aegis): complete Phase A platform decoupling"
```
