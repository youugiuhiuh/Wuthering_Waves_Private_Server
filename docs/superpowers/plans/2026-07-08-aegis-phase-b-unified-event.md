# Aegis Phase B: Unified Event Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify event reception, authorization, command processing, and destruct flow across all platforms into a shared `dispatch_event()` entry point.

**Architecture:** Each platform normalizes native events into `BotEvent` enum, then calls `shared::dispatch_event(event, &state)`. The shared layer handles authorization, destruct flow interception, command dispatch, and state operations — all using `BotAdapter` trait, no platform-specific types.

**Tech Stack:** Rust 2024, tokio, async-trait, mockall, teloxide (Telegram only), matrix-sdk (Matrix only)

## Global Constraints

- `cargo fmt && cargo clippy -- -D warnings && cargo test` must pass after every task
- All shared-layer code must use `BotAdapter` trait, not platform-specific types (`Bot`, `ChatId`, `Message`)
- `BotAdapter::download_file()` used for all file downloads (no `bot.get_file()`)
- `Markup` / `InlineButton` used for all keyboard rendering (no `InlineKeyboardMarkup`)
- Existing 504+ tests must continue passing (no behavior regression)
- `#[expect(dead_code)]` only for items that will be used by future platforms (Discord/LINE)
- i18n via `rust_i18n::t!()` macro — all user-facing strings must use i18n
- Worktree: `.worktrees/aegis-phase-b-unified-event/` on branch `aegis-phase-b-unified-event`

---

## File Structure

### New Files
| File | Responsibility |
|------|----------------|
| `src/shared/dispatch.rs` | Unified event entry: auth check → destruct intercept → dispatch by type |
| `src/shared/commands.rs` | `BotCommand` handlers: Help, Start, Auth, Menu, SetSecurityFile |
| `src/shared/destruct.rs` | Destruct flow using BotAdapter (replaces `destruct_flow.rs`) |
| `src/shared/state_ops.rs` | State operation intercepts: lang, set_timeout, a_warp_add_input |

### Modified Files
| File | Changes |
|------|---------|
| `src/shared/types.rs` | Add `BotEvent`, `MessageEvent`, `CommandEvent`, `BotCommand` |
| `src/shared/mod.rs` | Declare new modules |
| `src/main.rs` | Remove Command enum, handle_command, process_auth_code, looks_like_totp_code; teloxide dispatcher converts to BotEvent |
| `src/main/runtime.rs` | Telegram/Matrix handlers produce BotEvent, call `shared::dispatch_event()` |
| `src/app/mod.rs` | Remove `destruct_flow` module (logic moves to shared) |
| `src/lib.rs` | May need `pub use` for new shared modules |

### Deleted Files
| File | Reason |
|------|--------|
| `src/app/destruct_flow.rs` | Logic moves to `src/shared/destruct.rs` |
| `src/adapters/matrix/handlers.rs` | Matrix command dispatch replaced by unified system |

---

## Task 1: Extend types.rs with BotEvent, MessageEvent, CommandEvent, BotCommand

**Files:**
- Modify: `src/shared/types.rs`
- Modify: `src/shared/mod.rs`

**Interfaces:**
- Produces: `BotEvent`, `MessageEvent`, `CommandEvent`, `BotCommand` in `aegis::shared::types`
- Consumes: existing `CallbackEvent`, `TargetId`, `MessageId`, `BotAdapter`

- [ ] **Step 1.1: Write failing test for BotEvent construction**

```rust
// In src/shared/types.rs, add test at bottom:
#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::adapters::common::Markup;

    #[test]
    fn message_event_constructs() {
        // MessageEvent is a plain struct — verify fields compile
        let _ = MessageEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            user_id: 42,
            text: Some("hello".into()),
            file_id: None,
            reply_to_text: None,
        };
    }

    #[test]
    fn command_event_constructs() {
        let _ = CommandEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            user_id: 42,
            command: BotCommand::Help,
        };
    }

    #[test]
    fn bot_command_auth_carries_code() {
        let cmd = BotCommand::Auth { code: "123456".into() };
        assert!(matches!(cmd, BotCommand::Auth { ref code } if code == "123456"));
    }
}
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cd rust/aegis && cargo test shared::types::event_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `MessageEvent`, `CommandEvent`, `BotCommand` not defined

- [ ] **Step 1.3: Add types to types.rs**

```rust
// src/shared/types.rs — add after existing types

pub enum BotEvent {
    Message(MessageEvent),
    Callback(CallbackEvent),
    Command(CommandEvent),
}

impl BotEvent {
    pub fn user_id(&self) -> i64 {
        match self {
            BotEvent::Message(m) => m.user_id,
            BotEvent::Callback(c) => c.user_id.parse().unwrap_or(0),
            BotEvent::Command(c) => c.user_id,
        }
    }

    pub fn adapter(&self) -> &Arc<dyn BotAdapter> {
        match self {
            BotEvent::Message(m) => &m.adapter,
            BotEvent::Callback(c) => &c.adapter,
            BotEvent::Command(c) => &c.adapter,
        }
    }

    pub fn target(&self) -> &TargetId {
        match self {
            BotEvent::Message(m) => &m.target,
            BotEvent::Callback(c) => &c.target,
            BotEvent::Command(c) => &c.target,
        }
    }
}

pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub reply_to_text: Option<String>,
}

pub struct CommandEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub command: BotCommand,
}

pub enum BotCommand {
    Help,
    Start,
    Menu,
    Auth { code: String },
    SetSecurityFile,
}
```

- [ ] **Step 1.4: Run test to verify it passes**

Run: `cd rust/aegis && cargo test shared::types::event_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — 3 tests

- [ ] **Step 1.5: Run full suite + lint**

Run: `cd rust/aegis && cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 1.6: Commit**

```bash
git add -A && git commit -m "feat(aegis): add BotEvent, MessageEvent, CommandEvent, BotCommand types"
```

---

## Task 2: Create shared/state_ops.rs — state operation intercepts

**Files:**
- Create: `src/shared/state_ops.rs`
- Modify: `src/shared/mod.rs`
- Modify: `src/adapters/telegram/handlers/callback.rs` (remove inline intercepts)

**Interfaces:**
- Consumes: `CallbackEvent`, `AppState`
- Produces: `state_ops::intercept(callback, state)` — handles lang/set_timeout/a_warp_add_input

- [ ] **Step 2.1: Write failing test**

```rust
// src/shared/state_ops.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::core::totp::TotpManager;
    use crate::shared::types::CallbackEvent;
    use crate::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId, MockBotAdapter};
    use std::sync::Arc;
    use std::time::Instant;

    fn make_state() -> AppState {
        // ... same pattern as state.rs tests
        AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        )
    }

    #[tokio::test]
    async fn intercept_set_timeout_persists() {
        let state = make_state();
        let event = CallbackEvent {
            adapter: Arc::new(MockBotAdapter::new()) as Arc<dyn BotAdapter>,
            target: TargetId("123".into()),
            user_id: "42".into(),
            msg_id: MessageId("1".into()),
            data: "set_timeout:3600".into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        };
        intercept(&event, &state).await;
        assert_eq!(state.session_timeout_secs().await, 3600);
    }
}
```

- [ ] **Step 2.2: Run test — expect FAIL (intercept not defined)**

Run: `cd rust/aegis && cargo test shared::state_ops -- --nocapture 2>&1 | tail -10`

- [ ] **Step 2.3: Implement state_ops.rs**

```rust
// src/shared/state_ops.rs
use std::time::Instant;
use crate::adapters::common::BotAdapter;
use crate::shared::types::CallbackEvent;
use crate::app::state::AppState;
use crate::core::i18n;
use crate::bootstrap::BotSettings;

pub async fn intercept(cb: &CallbackEvent, state: &AppState) {
    let data = cb.data.as_str();

    if data.starts_with("lang:") {
        handle_lang(cb, state).await;
        return;
    }

    if data.starts_with("set_timeout:") {
        handle_set_timeout(cb, state).await;
        return;
    }

    if data == "a_warp_add_input" {
        state.start_warp_input(cb.target.0.clone(), Instant::now()).await;
        return;
    }
}

async fn handle_set_timeout(cb: &CallbackEvent, state: &AppState) {
    let secs: u64 = cb.data
        .strip_prefix("set_timeout:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(600);
    state.set_session_timeout_secs(secs).await;
    let settings = BotSettings { session_timeout_secs: secs };
    if let Err(e) = settings.save() {
        log::error!("保存会话设置失败: {}", e);
    }
}

async fn handle_lang(cb: &CallbackEvent, state: &AppState) {
    let lang = match cb.data.as_str() {
        "lang:zh" => i18n::Lang::Zh,
        "lang:en" => i18n::Lang::En,
        "lang:ja" => i18n::Lang::Ja,
        _ => return,
    };
    i18n::set_lang(lang);
    state.set_lang(lang).await;
    state.mark_lang_configured().await;
    i18n::mark_lang_configured();
    // Note: timedatectl and apt-daily timer stay in Telegram layer
    // (system operations that don't belong in shared)
}
```

- [ ] **Step 2.4: Add to shared/mod.rs**

```rust
// src/shared/mod.rs — add:
pub(crate) mod state_ops;
```

- [ ] **Step 2.5: Run test — expect PASS**

- [ ] **Step 2.6: Run full suite + lint**

- [ ] **Step 2.7: Commit**

```bash
git add -A && git commit -m "feat(aegis): add shared state_ops for lang/set_timeout/warp intercepts"
```

---

## Task 3: Create shared/destruct.rs — destruct flow with BotAdapter

**Files:**
- Create: `src/shared/destruct.rs`
- Modify: `src/shared/mod.rs`

**Interfaces:**
- Consumes: `BotEvent`, `MessageEvent`, `CallbackEvent`, `AppState`, `BotAdapter`
- Produces: `destruct::intercept_message(msg, state) -> Result<FlowOutcome>`, `destruct::intercept_callback(cb, state) -> Result<FlowOutcome>`

**IMPORTANT:** The pure logic layer (`process_destruct_message`) in `destruct_flow.rs` is already platform-agnostic. Only migrate the UI layer (send_message, InlineKeyboardMarkup → Markup, bot.get_file → adapter.download_file).

- [ ] **Step 3.1: Write failing test**

```rust
// src/shared/destruct.rs
#[cfg(test)]
mod tests {
    use super::*;
    // Test: intercept_callback with "a_destroy_ask" begins destruct
    // Test: intercept_callback with "a_destroy_cancel" cancels destruct
    // Test: intercept_message with TOTP code in destruct state advances step
    // Test: FlowOutcome::NotReturned when no destruct in progress
}
```

- [ ] **Step 3.2: Run test — expect FAIL**

- [ ] **Step 3.3: Implement destruct.rs**

Migrate all functions from `destruct_flow.rs`:
- `handle_message_flow(bot, msg, user_id, state)` → `intercept_message(msg: &MessageEvent, state: &AppState) -> Result<FlowOutcome>`
- `handle_callback_timeout(bot, q, chat_id, msg_id, state)` → `intercept_callback_timeout(cb: &CallbackEvent, state: &AppState) -> Result<FlowOutcome>`
- `handle_callback_action(bot, q, data, chat_id, msg_id, state)` → `intercept_callback_action(cb: &CallbackEvent, state: &AppState) -> Result<FlowOutcome>`

Key transformations:
- `bot.send_message(chat_id, text)` → `adapter.send_message(&target, MessageContent { text, markup })`
- `bot.edit_message_text(chat_id, msg_id, text)` → `adapter.edit_message(&target, &msg_id, MessageContent { text, markup })`
- `InlineKeyboardMarkup::new(vec![...])` → `Markup { buttons: vec![vec![InlineButton { text, data }]] }`
- `bot.get_file(fid)` + `bot.download_file()` → `adapter.download_file(&fid)`
- `bot.answer_callback_query(q.id)` → `adapter.answer_callback(&callback_id, "")`
- `msg.text()` → `msg.text.as_deref()`
- `msg.document()` / `msg.photo()` → `msg.file_id.as_ref()`
- `chat_id.0.to_string()` → `cb.target.0.clone()`
- `Instant::now()` stays as-is

Define `FlowOutcome`:
```rust
pub enum FlowOutcome {
    Handled,
    NotHandled,
}
```

- [ ] **Step 3.4: Add to shared/mod.rs**

```rust
pub(crate) mod destruct;
```

- [ ] **Step 3.5: Run test — expect PASS**

- [ ] **Step 3.6: Run full suite + lint**

- [ ] **Step 3.7: Commit**

```bash
git add -A && git commit -m "feat(aegis): add shared destruct flow with BotAdapter"
```

---

## Task 4: Create shared/commands.rs — unified command handlers

**Files:**
- Create: `src/shared/commands.rs`
- Modify: `src/shared/mod.rs`

**Interfaces:**
- Consumes: `CommandEvent`, `BotCommand`, `AppState`, `BotAdapter`
- Produces: `commands::handle(cmd: CommandEvent, state: &AppState) -> Result<()>`

- [ ] **Step 4.1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    // Test: BotCommand::Help sends help text
    // Test: BotCommand::Start sends welcome
    // Test: BotCommand::Auth { code } calls process_auth_code
    // Test: BotCommand::Menu calls send_main_menu when authorized
    // Test: BotCommand::Menu sends "auth required" when not authorized
    // Test: BotCommand::SetSecurityFile downloads file and hashes it
}
```

- [ ] **Step 4.2: Run test — expect FAIL**

- [ ] **Step 4.3: Implement commands.rs**

```rust
// src/shared/commands.rs
use crate::adapters::common::{BotAdapter, MessageContent, TargetId};
use crate::app::auth;
use crate::app::state::AppState;
use crate::shared::types::{BotCommand, CommandEvent};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub async fn handle(cmd: CommandEvent, state: &AppState) -> Result<()> {
    match cmd.command {
        BotCommand::Help => {
            cmd.adapter.send_message(&cmd.target, MessageContent {
                text: rust_i18n::t!("help.text").into_owned(),
                markup: None,
            }).await?;
        }
        BotCommand::Start => {
            cmd.adapter.send_message(&cmd.target, MessageContent {
                text: format!("{}\n\n{}",
                    rust_i18n::t!("welcome.title"),
                    rust_i18n::t!("welcome.prompt")),
                markup: None,
            }).await?;
        }
        BotCommand::Auth { code } => {
            let _ = auth::process_auth_code(
                &*cmd.adapter,
                &cmd.target,
                cmd.user_id,
                &code,
                state,
                5,                                    // TOTP_FAIL_MAX
                std::time::Duration::from_secs(600),  // TOTP_FAIL_WINDOW
                &[                                    // LOCKOUT_DURATIONS
                    std::time::Duration::from_secs(15 * 60),
                    std::time::Duration::from_secs(60 * 60),
                    std::time::Duration::from_secs(24 * 60 * 60),
                    std::time::Duration::from_secs(48 * 60 * 60),
                ],
            ).await;
        }
        BotCommand::Menu => {
            if !state.is_authorized(cmd.user_id).await {
                cmd.adapter.send_message(&cmd.target, MessageContent {
                    text: rust_i18n::t!("auth.required").into_owned(),
                    markup: None,
                }).await?;
                return Ok(());
            }
            crate::shared::handlers::menu::send_main_menu(&*cmd.adapter, &cmd.target).await?;
        }
        BotCommand::SetSecurityFile => {
            if !state.is_recently_authenticated(cmd.user_id).await {
                cmd.adapter.send_message(&cmd.target, MessageContent {
                    text: rust_i18n::t!("auth.recent_auth_required").into_owned(),
                    markup: None,
                }).await?;
                return Ok(());
            }
            // Note: file_id comes from MessageEvent, not CommandEvent
            // This command is actually triggered from MessageEvent with file
            // See dispatch.rs for routing logic
            cmd.adapter.send_message(&cmd.target, MessageContent {
                text: rust_i18n::t!("bot_commands.security_file_prompt").into_owned(),
                markup: None,
            }).await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4.4: Add to shared/mod.rs**

```rust
pub(crate) mod commands;
```

- [ ] **Step 4.5: Run test — expect PASS**

- [ ] **Step 4.6: Run full suite + lint**

- [ ] **Step 4.7: Commit**

```bash
git add -A && git commit -m "feat(aegis): add unified command handlers"
```

---

## Task 5: Create shared/dispatch.rs — unified event entry

**Files:**
- Create: `src/shared/dispatch.rs`
- Modify: `src/shared/mod.rs`

**Interfaces:**
- Consumes: `BotEvent`, `AppState`, `destruct`, `commands`, `state_ops`, `handlers`
- Produces: `dispatch::dispatch_event(event: BotEvent, state: &AppState) -> Result<()>`

- [ ] **Step 5.1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    // Test: Command event with Help → sends help text
    // Test: Callback event → state_ops intercept + handlers dispatch
    // Test: Message event in destruct state → destruct intercept handles it
    // Test: Unauthorized callback → auth denied message
    // Test: TOTP code message when unauthorized → auth processing
}
```

- [ ] **Step 5.2: Run test — expect FAIL**

- [ ] **Step 5.3: Implement dispatch.rs**

```rust
// src/shared/dispatch.rs
use crate::adapters::common::{BotAdapter, MessageContent};
use crate::app::state::AppState;
use crate::shared::types::{BotEvent, BotCommand, CallbackEvent, CommandEvent, MessageEvent};
use crate::shared::{commands, destruct, handlers, state_ops};
use anyhow::Result;

pub async fn dispatch_event(event: BotEvent, state: &AppState) -> Result<()> {
    // 1. Destruct flow interception (checks timeout, handles in-progress destruct)
    match &event {
        BotEvent::Message(msg) => {
            if destruct::intercept_message(msg, state).await? == destruct::FlowOutcome::Handled {
                return Ok(());
            }
        }
        BotEvent::Callback(cb) => {
            if destruct::intercept_callback(cb, state).await? == destruct::FlowOutcome::Handled {
                return Ok(());
            }
        }
        BotEvent::Command(_) => {}
    }

    // 2. Authorization check
    if !check_auth(&event, state).await {
        return Ok(());
    }

    // 3. Dispatch by event type
    match event {
        BotEvent::Command(cmd) => {
            commands::handle(cmd, state).await?;
        }
        BotEvent::Message(msg) => {
            handle_message(msg, state).await?;
        }
        BotEvent::Callback(cb) => {
            // State operations (lang, set_timeout, warp input)
            state_ops::intercept(&cb, state).await;
            // Shared callback dispatch (from Phase A)
            match handlers::dispatch(&cb).await? {
                Some(action) => {
                    // Handle HandlerAction::Redirect if needed
                    let _ = action;
                }
                None => {}
            }
        }
    }
    Ok(())
}

async fn check_auth(event: &BotEvent, state: &AppState) -> bool {
    let user_id = event.user_id();
    if !state.is_admin_user(user_id) {
        return false;
    }
    match event {
        BotEvent::Command(CommandEvent { command: BotCommand::Auth { .. }, .. }) => true,
        BotEvent::Command(CommandEvent { command: BotCommand::Help | BotCommand::Start, .. }) => true,
        BotEvent::Message(msg) => {
            // TOTP codes allowed when not authorized (login attempt)
            if let Some(ref text) = msg.text {
                if is_totp_code(text) && !state.is_authorized(user_id).await {
                    return true;
                }
            }
            state.is_authorized(user_id).await
        }
        _ => state.is_authorized(user_id).await,
    }
}

fn is_totp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

async fn handle_message(msg: MessageEvent, state: &AppState) -> Result<()> {
    // Delegate to shared message handler (from Phase A)
    // If it returns NeedsDestruct, destruct intercept already handled it above
    let action = crate::shared::handlers::message::handle_message(
        &*msg.adapter,
        &msg.target,
        msg.text.as_deref(),
        msg.file_id.is_some(),
        state,  // AppState implements MessageState trait
    ).await?;

    // If message handler says NeedsDestruct but destruct didn't intercept,
    // check for TOTP code
    if let crate::shared::handlers::message::MessageAction::NeedsDestruct = action {
        if let Some(ref text) = msg.text {
            let code = text.trim();
            if is_totp_code(code) && !state.is_authorized(msg.user_id).await {
                // Process as auth code
                let _ = crate::app::auth::process_auth_code(
                    &*msg.adapter,
                    &msg.target,
                    msg.user_id,
                    code,
                    state,
                    5,
                    std::time::Duration::from_secs(600),
                    &[
                        std::time::Duration::from_secs(15 * 60),
                        std::time::Duration::from_secs(60 * 60),
                        std::time::Duration::from_secs(24 * 60 * 60),
                        std::time::Duration::from_secs(48 * 60 * 60),
                    ],
                ).await;
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 5.4: Add to shared/mod.rs**

```rust
pub(crate) mod dispatch;
```

- [ ] **Step 5.5: Run test — expect PASS**

- [ ] **Step 5.6: Run full suite + lint**

- [ ] **Step 5.7: Commit**

```bash
git add -A && git commit -m "feat(aegis): add unified event dispatch"
```

---

## Task 6: Wire Telegram events to BotEvent in runtime.rs

**Files:**
- Modify: `src/main/runtime.rs`
- Modify: `src/main.rs` (remove Command enum, handle_command)

**Interfaces:**
- Consumes: `shared::dispatch::dispatch_event`, `shared::types::BotEvent`
- Produces: Telegram dispatcher converts teloxide updates to BotEvent

- [ ] **Step 6.1: Convert Telegram command handler**

Remove `Command` enum, `handle_command`, `looks_like_totp_code`, `process_auth_code` from `main.rs`.
Replace teloxide dispatcher in `runtime.rs` with BotEvent-producing handlers:

```rust
// src/main/runtime.rs — Telegram dispatcher section

if enable_telegram {
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<TeloxideCommand>()
                .endpoint(|bot: Bot, msg: Message, state: Arc<AppState>| async move {
                    let cmd = parse_teloxide_command(&msg);
                    let event = BotEvent::Command(CommandEvent {
                        adapter: state.adapter.clone(),
                        target: TargetId(msg.chat.id.0.to_string()),
                        user_id: msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0),
                        command: cmd,
                    });
                    let _ = dispatch_event(event, &state).await;
                    Ok(())
                }),
        )
        .branch(
            Update::filter_message().endpoint(|bot: Bot, msg: Message, state: Arc<AppState>| async move {
                let event = BotEvent::Message(MessageEvent {
                    adapter: state.adapter.clone(),
                    target: TargetId(msg.chat.id.0.to_string()),
                    user_id: msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0),
                    text: msg.text().map(|s| s.to_string()),
                    file_id: msg.document().map(|d| d.file.id.clone())
                        .or_else(|| msg.photo().and_then(|p| p.last().map(|ph| ph.file.id.clone()))),
                    reply_to_text: msg.reply_to_message().and_then(|r| r.text().map(|s| s.to_string())),
                });
                let _ = dispatch_event(event, &state).await;
                Ok(())
            }),
        )
        .branch(
            Update::filter_callback_query().endpoint(|q: CallbackQuery, state: Arc<AppState>| async move {
                // Existing callback.rs logic stays, but calls dispatch_event
                // callback.rs already calls shared::handlers::dispatch
                // This branch delegates to callback::handle_callback (unchanged for now)
                callback::handle_callback(/* ... */).await
            }),
        );
    // ...
}
```

Define a thin `TeloxideCommand` enum for teloxide's `filter_command` macro (just for parsing, then convert to `BotCommand`):

```rust
#[derive(BotCommands, Clone)]
enum TeloxideCommand {
    Help,
    Start,
    Menu,
    Auth(String),
    SetSecurityFile,
}

fn parse_teloxide_command(msg: &Message) -> BotCommand {
    // Convert teloxide command to BotCommand
    // This is called from the endpoint after filter_command
}
```

- [ ] **Step 6.2: Verify Telegram callback.rs still works**

`callback.rs` already calls `shared::handlers::dispatch()`. It also handles `lang:`, `set_timeout:`, `a_warp_add_input` inline. These should be moved to `state_ops::intercept()` but that requires `AppState` access in the shared dispatch.

For now, keep `callback.rs` as-is — it already works from Phase A. The unified dispatch is used for Message and Command events. Callback events still go through the existing path.

- [ ] **Step 6.3: Convert Matrix event handler**

```rust
// src/main/runtime.rs — Matrix section

client.add_event_handler(
    move |event: OriginalSyncRoomMessageEvent, room: MatrixRoom, _client: MatrixClient| {
        let state = matrix_state.clone();
        async move {
            if room.room_id().as_str() != target.0.as_str() { return; }
            let user_id = parse_user_id(event.sender.as_str());
            if !state.is_admin_user(user_id) { return; }

            let text = event.content.body().trim().to_string();

            // Try to parse as command, otherwise treat as message
            let event = if let Some(cmd) = parse_matrix_command(&text) {
                BotEvent::Command(CommandEvent {
                    adapter: state.adapter.clone(),
                    target: target.clone(),
                    user_id,
                    command: cmd,
                })
            } else {
                BotEvent::Message(MessageEvent {
                    adapter: state.adapter.clone(),
                    target: target.clone(),
                    user_id,
                    text: Some(text),
                    file_id: None,
                    reply_to_text: None,
                })
            };

            let _ = dispatch_event(event, &state).await;
        }
    },
);
```

- [ ] **Step 6.4: Run full suite + lint**

- [ ] **Step 6.5: Commit**

```bash
git add -A && git commit -m "feat(aegis): wire Telegram and Matrix events to unified dispatch"
```

---

## Task 7: Remove old destruct_flow.rs and simplify main.rs

**Files:**
- Delete: `src/app/destruct_flow.rs`
- Modify: `src/app/mod.rs` (remove `pub mod destruct_flow`)
- Modify: `src/main.rs` (remove process_auth_code, looks_like_totp_code, Command enum, handle_command, notify_online, notify_upgrade_success, notify_bbr3_reboot_result — move notify_* to shared or keep in main but using BotAdapter)

**IMPORTANT:** `notify_online`, `notify_upgrade_success`, `notify_bbr3_reboot_result` already use `BotAdapter` (they were migrated in Phase A). They can stay in `main.rs` or move to `shared/notifications.rs`.

- [ ] **Step 7.1: Delete destruct_flow.rs**

```bash
rm src/app/destruct_flow.rs
```

Update `src/app/mod.rs`:
```rust
pub mod auth;
pub mod batch_handler;  // if still exists
pub mod state;
// Remove: pub mod destruct_flow;
```

- [ ] **Step 7.2: Remove old command infrastructure from main.rs**

Remove from `main.rs`:
- `Command` enum (replaced by `TeloxideCommand` in runtime.rs)
- `handle_command` function (replaced by `shared::commands::handle`)
- `looks_like_totp_code` function (moved to `shared::dispatch`)
- `process_auth_code` function (logic now in `shared::dispatch` calling `app::auth`)
- `TOTP_FAIL_MAX`, `TOTP_FAIL_WINDOW`, `LOCKOUT_DURATIONS` constants (move to `shared::dispatch` or `app::auth`)

- [ ] **Step 7.3: Update callback.rs to use state_ops**

In `src/adapters/telegram/handlers/callback.rs`, replace inline `lang:` handler and `set_timeout:` / `a_warp_add_input` intercepts with `state_ops::intercept()`:

```rust
// Before shared dispatch:
aegis::shared::state_ops::intercept(&event, &state).await;
```

Then remove the inline handlers from callback.rs.

**Note:** The `lang:` handler in callback.rs also does `timedatectl` and `apt-daily` timer setup — these are system operations. Move them to a separate function in the Telegram layer that `state_ops::handle_lang` calls via a callback, OR keep `lang:` handling in callback.rs for now and only move `set_timeout:` and `a_warp_add_input` to state_ops.

- [ ] **Step 7.4: Run full suite + lint**

- [ ] **Step 7.5: Commit**

```bash
git add -A && git commit -m "refactor(aegis): remove old destruct_flow, simplify main.rs"
```

---

## Task 8: Remove matrix/handlers.rs — unified dispatch replaces it

**Files:**
- Delete: `src/adapters/matrix/handlers.rs`
- Modify: `src/adapters/matrix/mod.rs` (remove `pub mod commands` or keep for parsing only)
- Modify: `src/main.rs` (remove `mod matrix_handlers`)

- [ ] **Step 8.1: Check what matrix_handlers::dispatch does**

Read `src/adapters/matrix/handlers.rs` and `src/adapters/matrix/commands.rs`. The Matrix command parser (`commands::parse`) converts text to a Matrix-specific `Command` enum. The `handlers::dispatch` function dispatches these commands.

With unified dispatch, Matrix text is parsed into `BotCommand` directly, and `shared::commands::handle` processes it. The old Matrix command system is replaced.

- [ ] **Step 8.2: Create matrix command parser → BotCommand**

In `src/adapters/matrix/commands.rs`, add a function that parses text to `BotCommand`:

```rust
pub fn parse_to_bot_command(text: &str) -> Option<BotCommand> {
    let text = text.trim();
    if text == "/help" || text == "/h" { return Some(BotCommand::Help); }
    if text == "/start" { return Some(BotCommand::Start); }
    if text == "/menu" { return Some(BotCommand::Menu); }
    if text == "/setsecurityfile" { return Some(BotCommand::SetSecurityFile); }
    if let Some(code) = text.strip_prefix("/auth ") {
        return Some(BotCommand::Auth { code: code.trim().to_string() });
    }
    None
}
```

- [ ] **Step 8.3: Delete matrix/handlers.rs**

```bash
rm src/adapters/matrix/handlers.rs
```

Update `src/adapters/matrix/mod.rs`:
```rust
pub mod adapter;
pub mod commands;
pub use adapter::MatrixAdapter;
// Remove: pub mod handlers; (if it existed)
```

Update `src/main.rs`:
```rust
// Remove: #[path = "adapters/matrix/handlers.rs"] mod matrix_handlers;
```

- [ ] **Step 8.4: Update runtime.rs Matrix section**

Use `parse_to_bot_command` in the Matrix event handler (already done in Task 6, just verify).

- [ ] **Step 8.5: Run full suite + lint**

- [ ] **Step 8.6: Commit**

```bash
git add -A && git commit -m "refactor(aegis): remove matrix-specific command dispatch, use unified BotCommand"
```

---

## Task 9: Final cleanup — dead code removal and lint

**Files:**
- Modify: various (remove `#[expect(dead_code)]` that are now used, remove unused imports)

- [ ] **Step 9.1: Remove now-unused `#[expect(dead_code)]` annotations**

After Phase B, some items that were dead in Phase A are now used:
- `AppState::start_warp_input` — now called from `state_ops::intercept`
- `AppState::set_session_timeout_secs` — now called from `state_ops::intercept`
- `BotSettings::save` — now called from `state_ops::handle_set_timeout`
- `ScheduleFrequency`, `ScheduleInputState` — still dead (custom wizard removed), keep `#[expect(dead_code)]`

- [ ] **Step 9.2: Remove unused imports**

Run `cargo clippy -- -D warnings` and fix all unused import warnings.

- [ ] **Step 9.3: Final test pass**

Run: `cd rust/aegis && cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures, clippy clean

- [ ] **Step 9.4: Commit**

```bash
git add -A && git commit -m "chore(aegis): Phase B final cleanup — remove dead code, fix lint"
```

---

## Verification Checklist

After all tasks complete:

- [ ] `cargo fmt` passes
- [ ] `cargo clippy -- -D warnings` passes (0 warnings)
- [ ] `cargo test` passes (all existing + new tests)
- [ ] No teloxide types (`Bot`, `ChatId`, `Message`, `InlineKeyboardMarkup`) in `src/shared/`
- [ ] No direct `bot.get_file()` / `bot.download_file()` calls (all via `adapter.download_file()`)
- [ ] `main.rs` no longer defines `Command` enum or `handle_command`
- [ ] `destruct_flow.rs` deleted (logic in `shared/destruct.rs`)
- [ ] `matrix/handlers.rs` deleted (logic in `shared/commands.rs`)
- [ ] Telegram dispatcher produces `BotEvent` and calls `shared::dispatch_event()`
- [ ] Matrix event handler produces `BotEvent` and calls `shared::dispatch_event()`
- [ ] Authorization check unified in `shared/dispatch.rs`
- [ ] Destruct flow uses `BotAdapter` (no teloxide types)
- [ ] File download uses `adapter.download_file()` (no `bot.get_file()`)
