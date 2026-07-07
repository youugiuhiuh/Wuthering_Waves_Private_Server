# Decouple Telegram from app Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove `teloxide` type dependencies from `app/batch_handler.rs` and `app/destruct_flow.rs`, making the app layer platform-agnostic.

**Architecture:** Business logic in `app/` speaks only `BotAdapter` + platform-agnostic `DestructInput`/`DestructOutput`. Telegram-specific UI rendering stays in `adapters/telegram/handlers/destruct.rs`.

**Tech Stack:** Rust, teloxide, tokio, async_trait, rust_i18n

## Global Constraints

- `BotAdapter` trait stays unchanged.
- Markup conversion stays in `adapters/telegram/adapter.rs`.
- Only edit files listed below.
- Task 3 removes the three handler functions from `app/destruct_flow.rs` — do NOT remove them in Task 2 (tests must pass mid-way).
- All `a_destroy_*` callback data strings become `pub const` in `app/destruct_flow.rs`.
- The pre-existing test failure `core::system::tests::test_version_check_with_env` is known and unrelated.

---
### Task 1: Refactor `app/batch_handler.rs` to use `&TargetId`

**Files:**
- Modify: `src/app/batch_handler.rs` (signature + tests)
- Modify: `src/adapters/telegram/handlers/singbox.rs` (2 call sites)

- [ ] **Step 1: Change signature and remove `ChatId` import**

In `src/app/batch_handler.rs`:
- Delete the line `use teloxide::types::ChatId;`
- Change the function signature parameter from `chat_id: ChatId` to `target: &TargetId`
- Remove `let target = TargetId(chat_id.0.to_string());` (line 19) — `target` is now the parameter

```rust
/// Send SingBox batch creation results through the adapter (supports routing):
/// header message, chunked link messages, summary message,
/// then best-effort auto-delete after 60 seconds.
pub async fn send_singbox_batch_result(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    protocol_name: &str,
    result: &BatchCreationResult,
) -> anyhow::Result<()> {
    let mut message_ids: Vec<String> = Vec::new();
    ...
```

- [ ] **Step 2: Compile to check**

```bash
cargo check -p aegis 2>&1 | tail -15
```

- [ ] **Step 3: Update callers in `singbox.rs`**

In `src/adapters/telegram/handlers/singbox.rs`, find the two call sites (lines 374 and 436 approx). Change:

```rust
send_singbox_batch_result(
    adapter.clone(),
    chat_id_clone,
    "Hysteria2",
    &result,
)
```
to:
```rust
send_singbox_batch_result(
    adapter.clone(),
    &TargetId(chat_id_clone.0.to_string()),
    "Hysteria2",
    &result,
)
```

Repeat for the second call site (TUIC, swap protocol name).

`TargetId` is already imported in `singbox.rs` (line 3).

- [ ] **Step 4: Compile again**

```bash
cargo check -p aegis 2>&1 | tail -15
```

- [ ] **Step 5: Update tests in `app/batch_handler.rs`**

Replace all `ChatId(N)` with `TargetId("N".to_string())` in test function calls. Four occurrences around lines 117, 131, 145, 161.

```rust
// Before:
send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
// After:
send_singbox_batch_result(Arc::new(mock), &TargetId("1".to_string()), "hy2", &result)
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p aegis -- batch_handler 2>&1
```
Expected: `test result: ok. 4 passed; 0 failed`

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "refactor(aegis): replace ChatId with TargetId in send_singbox_batch_result"
```

---
### Task 2: Add platform-agnostic I/O types and `handle_input`

**Files:**
- Modify: `src/app/destruct_flow.rs`

- [ ] **Step 1: Remove Telegram imports**

Delete these 3 lines from `src/app/destruct_flow.rs`:
```rust
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
```

Add after `use crate::app::state::{AppState, DestructStep, TimeoutStatus};`:
```rust
use crate::adapters::common::Markup;
```

- [ ] **Step 2: Add new types and constants**

After `MessageFlowOutcome` enum (before `process_destruct_message`), add:

```rust
#[derive(Debug, Clone)]
pub struct ButtonSpec {
    pub text: String,
    pub action: String,
}

#[derive(Debug, Clone)]
pub enum DestructInput {
    Text(String),
    File(Vec<u8>),
    Button(String),
}

#[derive(Debug, Clone)]
pub enum DestructOutput {
    Prompt { text: String, buttons: Vec<Vec<ButtonSpec>> },
    Text(String),
    Execute,
    Noop,
}

pub const BTN_DESTROY_ASK: &str = "a_destroy_ask";
pub const BTN_DESTROY_CONFIRM: &str = "a_destroy_confirm";
pub const BTN_DESTROY_CANCEL: &str = "a_destroy_cancel";
pub const BTN_DESTROY_FINAL: &str = "a_destroy_final";
```

- [ ] **Step 3: Add `handle_input` function**

Add before `handle_message_flow`:

```rust
pub async fn handle_input(
    state: &Arc<AppState>,
    chat_id: &str,
    user_id: i64,
    input: DestructInput,
    now: Instant,
) -> (MessageFlowOutcome, Vec<DestructOutput>) {
    if !state.is_authorized(user_id).await {
        return (MessageFlowOutcome::Handled, vec![
            DestructOutput::Text(t!("auth.expired").to_string()),
        ]);
    }

    let Some(destruct_state) = state.destruct_snapshot(chat_id).await else {
        return (MessageFlowOutcome::NotHandled, vec![]);
    };

    match (destruct_state.step, &input) {
        // ── AwaitFirstTotp + text ──
        (DestructStep::AwaitFirstTotp, DestructInput::Text(code))
            if state.verify_totp(code.trim()) =>
        {
            state.confirm_first_destruct_totp(chat_id, code.trim(), now).await;
            let buttons = vec![
                vec![ButtonSpec {
                    text: t!("destruct.confirm_btn").to_string(),
                    action: BTN_DESTROY_CONFIRM.to_string(),
                }],
                vec![ButtonSpec {
                    text: t!("destruct.cancelled").to_string(),
                    action: BTN_DESTROY_CANCEL.to_string(),
                }],
            ];
            (MessageFlowOutcome::Handled, vec![
                DestructOutput::Prompt {
                    text: t!("destruct.title_2").to_string(),
                    buttons,
                },
            ])
        }

        // ── AwaitFirstTotp + invalid ──
        (DestructStep::AwaitFirstTotp, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

        // ── AwaitSecondTotp + valid ──
        (DestructStep::AwaitSecondTotp, DestructInput::Text(code))
            if state.verify_totp(code.trim()) =>
        {
            match state.confirm_second_destruct_totp(chat_id, code.trim(), now).await {
                Err(_) => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.security_warn").to_string()),
                ]),
                Ok(true) => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.title_4").to_string()),
                ]),
                Ok(false) => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.state_invalid").to_string()),
                ]),
            }
        }

        // ── AwaitSecondTotp + invalid ──
        (DestructStep::AwaitSecondTotp, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

        // ── Generic text match ──
        (_, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

        // ── Security file verification ──
        (DestructStep::AwaitSecurityFile, DestructInput::File(content)) => {
            let action = process_destruct_message(
                None,
                DestructStep::AwaitSecurityFile,
                state,
                state.self_destruct_key_hash().await.as_deref(),
                Some(content),
            )
            .await;
            match action {
                DestructMessageAction::FileVerified { hash_short } => {
                    if state.mark_destruct_file_verified(chat_id, now).await {
                        let buttons = vec![
                            vec![ButtonSpec {
                                text: t!("destruct.final_btn").to_string(),
                                action: BTN_DESTROY_FINAL.to_string(),
                            }],
                            vec![ButtonSpec {
                                text: t!("destruct.cancelled").to_string(),
                                action: BTN_DESTROY_CANCEL.to_string(),
                            }],
                        ];
                        (MessageFlowOutcome::Handled, vec![
                            DestructOutput::Prompt {
                                text: t!("destruct.file_verify_ok", "0" => hash_short).to_string(),
                                buttons,
                            },
                        ])
                    } else {
                        (MessageFlowOutcome::Handled, vec![])
                    }
                }
                DestructMessageAction::FileMismatch => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.file_verify_fail").to_string()),
                ]),
                DestructMessageAction::NoSecurityKey => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.no_security_file").to_string()),
                ]),
                _ => (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.file_send_prompt").to_string()),
                ]),
            }
        }

        // ── Btn: ask ──
        (_, DestructInput::Button(btn)) if btn == BTN_DESTROY_ASK => {
            state.begin_destruct(chat_id.to_string(), now).await;
            let buttons = vec![
                vec![ButtonSpec {
                    text: t!("destruct.cancelled").to_string(),
                    action: BTN_DESTROY_CANCEL.to_string(),
                }],
            ];
            (MessageFlowOutcome::Handled, vec![
                DestructOutput::Prompt {
                    text: t!("destruct.title_1").to_string(),
                    buttons,
                },
            ])
        }

        // ── Btn: cancel ──
        (_, DestructInput::Button(btn)) if btn == BTN_DESTROY_CANCEL => {
            state.cancel_destruct(chat_id).await;
            (MessageFlowOutcome::Handled, vec![
                DestructOutput::Text(t!("destruct.cancelled").to_string()),
            ])
        }

        // ── Btn: confirm first TOTP ──
        (DestructStep::AwaitConfirm, DestructInput::Button(btn))
            if btn == BTN_DESTROY_CONFIRM =>
        {
            if state.advance_destruct_step(
                chat_id,
                DestructStep::AwaitConfirm,
                DestructStep::AwaitSecondTotp,
                now,
            ).await {
                let buttons = vec![
                    vec![ButtonSpec {
                        text: t!("destruct.cancelled").to_string(),
                        action: BTN_DESTROY_CANCEL.to_string(),
                    }],
                ];
                (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Prompt {
                        text: t!("destruct.title_3").to_string(),
                        buttons,
                    },
                ])
            } else {
                (MessageFlowOutcome::Handled, vec![
                    DestructOutput::Text(t!("destruct.state_invalid").to_string()),
                ])
            }
        }

        // ── Btn: final execute ──
        (DestructStep::AwaitFinalConfirm, DestructInput::Button(btn))
            if btn == BTN_DESTROY_FINAL =>
        {
            let executor = state.self_destruct_executor();
            aegis::core::security::self_destruct::trigger(executor);
            state.cancel_destruct(chat_id).await;
            (MessageFlowOutcome::Handled, vec![
                DestructOutput::Text(t!("destruct.final_exec").to_string()),
                DestructOutput::Execute,
            ])
        }

        _ => (MessageFlowOutcome::NotHandled, vec![]),
    }
}
```

- [ ] **Step 4: Write failing test for `handle_input`**

Append to `#[cfg(test)] mod tests` at end of `app/destruct_flow.rs`:

```rust
#[tokio::test]
async fn handle_input_first_totp_valid_returns_prompt() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use futures_util::future::BoxFuture;
    use anyhow::Result;
    use aegis::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use async_trait::async_trait;

    struct TE;
    #[async_trait]
    impl BotAdapter for TE {
        fn platform(&self) -> Platform { Platform::Telegram }
        async fn send_message(&self, _: &TargetId, _: MessageContent) -> Result<MessageId> { Ok(MessageId("0".to_string())) }
        async fn edit_message(&self, _: &TargetId, _: &MessageId, _: MessageContent) -> Result<()> { Ok(()) }
        async fn delete_message(&self, _: &TargetId, _: &MessageId) -> Result<()> { Ok(()) }
    }
    struct SE;
    impl SelfDestructExecutor for SE {
        fn execute(&self) -> BoxFuture<'static, Result<()>> { Box::pin(async { Ok(()) }) }
    }

    let secret = TotpManager::generate_new_secret();
    let tm = TotpManager::new(&SecretString::from(secret.clone())).unwrap();
    let totp = tm.generate_current().unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    state.begin_destruct("chat1".to_string(), Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat1", 42,
        DestructInput::Text(totp),
        Instant::now(),
    ).await;

    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert_eq!(outputs.len(), 1);
    assert!(matches!(&outputs[0], DestructOutput::Prompt { .. }));
}

#[tokio::test]
async fn handle_input_invalid_totp_returns_text() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    state.begin_destruct("chat_fail".to_string(), Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_fail", 42,
        DestructInput::Text("000000".to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Text(_)));
}

#[tokio::test]
async fn handle_input_unauthorized_returns_expired() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    state.begin_destruct("chat_ua".to_string(), Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_ua", 999,
        DestructInput::Text("111111".to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Text(_)));
}

#[tokio::test]
async fn handle_input_no_destruct_returns_not_handled() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    let (outcome, outputs) = handle_input(
        &state, "no_destruct", 42,
        DestructInput::Text("111111".to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::NotHandled);
    assert!(outputs.is_empty());
}

#[tokio::test]
async fn handle_input_cancel_button_removes_destruct() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    state.begin_destruct("chat_cancel".to_string(), Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_cancel", 42,
        DestructInput::Button(BTN_DESTROY_CANCEL.to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Text(_)));
    assert!(state.destruct_snapshot("chat_cancel").await.is_none());
}

#[tokio::test]
async fn handle_input_ask_button_begins_destruct() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));

    let (outcome, outputs) = handle_input(
        &state, "chat_ask", 42,
        DestructInput::Button(BTN_DESTROY_ASK.to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Prompt { .. }));
    assert!(state.destruct_snapshot("chat_ask").await.is_some());
}

#[tokio::test]
async fn handle_input_file_verify_valid() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let content = b"test security file";
    let hash = hex::encode(sha2::Sha256::digest(content));
    let state = Arc::new(AppState::new(
        42, tm, Arc::new(SE), Some(hash), 600, Arc::new(TE), None,
    ));
    state.begin_destruct("chat_file".to_string(), Instant::now()).await;
    state.advance_destruct_step("chat_file", DestructStep::AwaitFirstTotp, DestructStep::AwaitSecurityFile, Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_file", 42,
        DestructInput::File(content.to_vec()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Prompt { .. }));
}

#[tokio::test]
async fn handle_input_file_verify_mismatch() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(
        42, tm, Arc::new(SE), Some("fake_hash".to_string()), 600, Arc::new(TE), None,
    ));
    state.begin_destruct("chat_mis".to_string(), Instant::now()).await;
    state.advance_destruct_step("chat_mis", DestructStep::AwaitFirstTotp, DestructStep::AwaitSecurityFile, Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_mis", 42,
        DestructInput::File(b"wrong content".to_vec()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    assert!(matches!(&outputs[0], DestructOutput::Text(_)));
}

#[tokio::test]
async fn handle_input_confirm_button_advances_step() {
    use secrecy::SecretString;
    use aegis::core::totp::TotpManager;
    let tm = TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap();
    let state = Arc::new(AppState::new(42, tm, Arc::new(SE), None, 600, Arc::new(TE), None));
    state.begin_destruct("chat_cfm".to_string(), Instant::now()).await;

    let (outcome, outputs) = handle_input(
        &state, "chat_cfm", 42,
        DestructInput::Button(BTN_DESTROY_CONFIRM.to_string()),
        Instant::now(),
    ).await;
    assert_eq!(outcome, MessageFlowOutcome::Handled);
    let snap = state.destruct_snapshot("chat_cfm").await.unwrap();
    assert_eq!(snap.step, DestructStep::AwaitSecondTotp);
}
```

- [ ] **Step 5: Run all destruct_flow tests**

```bash
cargo test -p aegis -- destruct_flow 2>&1
```
Expected: existing tests + 9 new tests pass (13 total in module).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(aegis): add platform-agnostic DestructInput/Output and handle_input"
```

---
### Task 3: Create `adapters/telegram/handlers/destruct.rs` and wire callers

**Files:**
- Create: `src/adapters/telegram/handlers/destruct.rs`
- Modify: `src/adapters/telegram/handlers/mod.rs` (add `pub mod destruct;`)
- Modify: `src/adapters/telegram/handlers/message.rs` (update destruct call)
- Modify: `src/adapters/telegram/handlers/callback.rs` (update destruct calls)
- Modify: `src/app/destruct_flow.rs` (remove old handler functions)

- [ ] **Step 1: Create `src/adapters/telegram/handlers/destruct.rs`**

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::destruct_flow::{
    self, ButtonSpec, DestructInput, DestructOutput, MessageFlowOutcome,
};
use crate::app::state::{AppState, TimeoutStatus};
use rust_i18n::t;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle_message_flow(
    bot: &Bot,
    msg: &Message,
    user_id: i64,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    use crate::app::destruct_flow::BTN_DESTROY_ASK;
    use crate::app::destruct_flow::BTN_DESTROY_CANCEL;
    let chat_id = msg.chat.id;
    let chat_id_str = chat_id.0.to_string();

    // Timeout check
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.send_message(chat_id, t!("destruct.timeout")).await?;
            return Ok(MessageFlowOutcome::Handled);
        }
        TimeoutStatus::NotTracked => return Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::Active => {}
    }

    // Extract content from message
    let input = if let Some(text) = msg.text() {
        DestructInput::Text(text.to_string())
    } else if let Some(doc) = msg.document() {
        let file = bot.get_file(doc.file.id.clone()).await?;
        let mut content = Vec::new();
        bot.download_file(&file.path, &mut content)
            .await
            .map_err(|e| std::io::Error::other(e))?;
        DestructInput::File(content)
    } else if let Some(photos) = msg.photo() {
        if let Some(p) = photos.last() {
            let file = bot.get_file(p.file.id.clone()).await?;
            let mut content = Vec::new();
            bot.download_file(&file.path, &mut content)
                .await
                .map_err(|e| std::io::Error::other(e))?;
            DestructInput::File(content)
        } else {
            return Ok(MessageFlowOutcome::NotHandled);
        }
    } else {
        return Ok(MessageFlowOutcome::NotHandled);
    };

    let (outcome, outputs) =
        destruct_flow::handle_input(state, &chat_id_str, user_id, input, Instant::now()).await;

    for output in outputs {
        render_message_output(bot, chat_id, &output).await?;
    }

    Ok(outcome)
}

async fn render_message_output(
    bot: &Bot,
    chat_id: ChatId,
    output: &DestructOutput,
) -> ResponseResult<()> {
    match output {
        DestructOutput::Prompt { text, buttons } => {
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(convert_buttons(buttons))
                .await?;
        }
        DestructOutput::Text(text) => {
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        DestructOutput::Execute | DestructOutput::Noop => {}
    }
    Ok(())
}

pub async fn handle_callback_timeout(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id_str = chat_id.0.to_string();
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.answer_callback_query(q.id.clone())
                .text(t!("destruct.callback_timeout"))
                .await?;
            bot.edit_message_text(chat_id, msg_id, t!("destruct.timeout"))
                .parse_mode(ParseMode::Html)
                .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        TimeoutStatus::Active => Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::NotTracked => Ok(MessageFlowOutcome::NotHandled),
    }
}

pub async fn handle_callback_action(
    bot: &Bot,
    q: &CallbackQuery,
    data: &str,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    use crate::app::destruct_flow::BTN_DESTROY_CANCEL;
    use crate::app::destruct_flow::BTN_DESTROY_ASK;
    use crate::app::destruct_flow::BTN_DESTROY_FINAL;
    let chat_id_str = chat_id.0.to_string();

    // ── Special case: cancel restores Telegram-specific menu ──
    if data == BTN_DESTROY_CANCEL {
        let cancelled = state.cancel_destruct(&chat_id_str).await;
        if cancelled {
            bot.send_message(chat_id, t!("destruct.cancelled")).await?;
        }
        let keyboard = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                t!("destruct.destroy_btn"),
                BTN_DESTROY_ASK.to_string(),
            )],
            vec![InlineKeyboardButton::callback(
                t!("menu.back_settings"),
                "m_settings",
            )],
        ]);
        bot.edit_message_text(
            chat_id,
            msg_id,
            format!(
                "{}\n\n{}",
                t!("menu.danger_zone"),
                t!("menu.danger_zone_desc")
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
        return Ok(MessageFlowOutcome::Handled);
    }

    // ── Generic: delegate to handle_input ──
    let user_id = q.from.id.0 as i64;
    let (outcome, outputs) = destruct_flow::handle_input(
        state,
        &chat_id_str,
        user_id,
        DestructInput::Button(data.to_string()),
        Instant::now(),
    ).await;

    for output in outputs {
        match &output {
            DestructOutput::Execute => {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.executing"))
                    .await?;
                bot.edit_message_text(chat_id, msg_id, t!("destruct.final_exec"))
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            DestructOutput::Prompt { text, buttons } => {
                bot.edit_message_text(chat_id, msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(convert_buttons(buttons))
                    .await?;
            }
            DestructOutput::Text(text) => {
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            DestructOutput::Noop => {}
        }
    }

    Ok(outcome)
}

fn convert_buttons(buttons: &[Vec<ButtonSpec>]) -> InlineKeyboardMarkup {
    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .iter()
        .map(|row| {
            row.iter()
                .map(|btn| InlineKeyboardButton::callback(&btn.text, &btn.action))
                .collect()
        })
        .collect();
    InlineKeyboardMarkup::new(rows)
}
```

- [ ] **Step 2: Update `mod.rs`**

Add `pub(crate) mod destruct;` after `pub mod context;` in `src/adapters/telegram/handlers/mod.rs`.

- [ ] **Step 3: Update `message.rs`**

Change:
```rust
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
```
to:
```rust
use crate::handlers::destruct;
use crate::handlers::destruct::MessageFlowOutcome;
```

Wait — `MessageFlowOutcome` is still defined in `crate::app::destruct_flow`. The new destruct.rs re-exports it via `use crate::app::destruct_flow::MessageFlowOutcome`. But `message.rs` can import directly from `crate::app::destruct_flow` for the type and from `crate::handlers::destruct` for the function.

Actually, let me keep the imports cleaner:

In `message.rs`:
```rust
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::handlers::destruct;
```

And change line 117 from:
```rust
if destruct_flow::handle_message_flow(&bot, &msg, user_id, &state).await?
```
to:
```rust
if destruct::handle_message_flow(&bot, &msg, user_id, &state).await?
```

- [ ] **Step 4: Update `callback.rs`**

Change:
```rust
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
```
to:
```rust
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::handlers::destruct;
```

Change line 97:
```rust
if destruct_flow::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
```
to:
```rust
if destruct::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
```

Change lines 127-135:
```rust
if destruct_flow::handle_callback_action(
    &bot,
    &q,
    data.as_str(),
    chat_id,
    msg_id,
    &state,
)
.await?
```
to:
```rust
if destruct::handle_callback_action(
    &bot,
    &q,
    data.as_str(),
    chat_id,
    msg_id,
    &state,
)
.await?
```

- [ ] **Step 5: Remove old handler functions from `app/destruct_flow.rs`**

Delete from `app/destruct_flow.rs`:
1. `pub async fn handle_message_flow(...)` (lines ~76-245)
2. `pub async fn handle_callback_timeout(...)` (lines ~247-272)
3. `pub async fn handle_callback_action(...)` (lines ~274-392)

Keep everything else: `process_destruct_message`, `DestructStep`, `DestructMessageAction`, `MessageFlowOutcome`, `DestructInput`, `DestructOutput`, `ButtonSpec`, `handle_input`, constants.

Also update the imports to only what the remaining code needs:

```rust
use std::sync::Arc;
use std::time::Instant;

use rust_i18n::t;
use sha2::Digest;

use crate::app::state::{AppState, DestructStep};
```

Remove these unused imports:
- `use teloxide::net::Download;`
- `use teloxide::prelude::*;`
- `use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};`
- `use crate::app::state::TimeoutStatus;`
- `use std::time::Duration;`

- [ ] **Step 6: Compile**

```bash
cargo check -p aegis 2>&1 | tail -20
```

- [ ] **Step 7: Run all tests**

```bash
cargo test -p aegis 2>&1 | tail -30
```
Expected: everything passes (except the pre-existing `test_version_check_with_env`).

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "refactor(aegis): move Telegram destruct handlers to adapters/telegram/handlers/destruct.rs"
```

---
## File Reference

Final state after all tasks:

| File | Platform dep? | Purpose |
|------|:---:|---------|
| `src/app/batch_handler.rs` | None | `send_singbox_batch_result(adapter, &TargetId, ...)` |
| `src/app/destruct_flow.rs` | None | `handle_input()` + types, `process_destruct_message()` |
| `src/adapters/telegram/handlers/destruct.rs` | teloxide | `handle_message_flow`, `handle_callback_*` with Teloxide types |
| `src/adapters/telegram/handlers/message.rs` | teloxide | Calls `destruct::handle_message_flow` |
| `src/adapters/telegram/handlers/callback.rs` | teloxide | Calls `destruct::handle_callback_*` |
| `src/adapters/telegram/handlers/mod.rs` | — | Adds `pub(crate) mod destruct;` |
