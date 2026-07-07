use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;
use sha2::Digest;

use crate::app::state::{AppState, DestructStep, TimeoutStatus};

// TEMPORARY: Telegram imports for old handler functions
// Will be removed in Task 3 when handlers move to adapters/telegram/handlers/destruct.rs
#[allow(unused_imports)]
use teloxide::net::Download;
#[allow(unused_imports)]
use teloxide::prelude::*;
#[allow(unused_imports)]
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestructMessageAction {
    ConfirmFirstTotp,
    AwaitingSecondTotp,
    VerifyFailed,
    AwaitingFile,
    FileVerified { hash_short: String },
    FileMismatch,
    NoSecurityKey,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageFlowOutcome {
    Handled,
    NotHandled,
}

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

/// Pure logic layer: given the current destruct step and user input,
/// decide what action should be taken.
/// Does NOT depend on teloxide Bot/Message types — directly testable.
pub async fn process_destruct_message(
    text: Option<&str>,
    step: DestructStep,
    state: &AppState,
    self_destruct_key_hash: Option<&str>,
    file_content: Option<&[u8]>,
) -> DestructMessageAction {
    match step {
        DestructStep::AwaitFirstTotp => match text {
            Some(code) if state.verify_totp(code.trim()) => DestructMessageAction::ConfirmFirstTotp,
            Some(_) => DestructMessageAction::VerifyFailed,
            None => DestructMessageAction::Noop,
        },
        DestructStep::AwaitSecondTotp => match text {
            Some(code) if state.verify_totp(code.trim()) => {
                DestructMessageAction::AwaitingSecondTotp
            }
            Some(_) => DestructMessageAction::VerifyFailed,
            None => DestructMessageAction::Noop,
        },
        DestructStep::AwaitSecurityFile => {
            if let Some(content) = file_content {
                let hash = hex::encode(sha2::Sha256::digest(content));
                match self_destruct_key_hash {
                    Some(correct) if hash == correct => {
                        let hash_short = if hash.len() > 12 {
                            format!("{}...{}", &hash[..8], &hash[hash.len() - 4..])
                        } else {
                            hash.clone()
                        };
                        DestructMessageAction::FileVerified { hash_short }
                    }
                    Some(_) => DestructMessageAction::FileMismatch,
                    None => DestructMessageAction::NoSecurityKey,
                }
            } else {
                DestructMessageAction::AwaitingFile
            }
        }
        DestructStep::AwaitConfirm | DestructStep::AwaitFinalConfirm => DestructMessageAction::Noop,
    }
}

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

    // Buttons that don't require existing destruct state
    if let DestructInput::Button(btn) = &input {
        if btn == BTN_DESTROY_ASK {
            state.begin_destruct(chat_id.to_string(), now).await;
            let buttons = vec![
                vec![ButtonSpec {
                    text: t!("destruct.cancelled").to_string(),
                    action: BTN_DESTROY_CANCEL.to_string(),
                }],
            ];
            return (MessageFlowOutcome::Handled, vec![
                DestructOutput::Prompt {
                    text: t!("destruct.title_1").to_string(),
                    buttons,
                },
            ]);
        }
        if btn == BTN_DESTROY_CANCEL {
            state.cancel_destruct(chat_id).await;
            return (MessageFlowOutcome::Handled, vec![
                DestructOutput::Text(t!("destruct.cancelled").to_string()),
            ]);
        }
    }

    let Some(destruct_state) = state.destruct_snapshot(chat_id).await else {
        return (MessageFlowOutcome::NotHandled, vec![]);
    };

    match (destruct_state.step, &input) {
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

        (DestructStep::AwaitFirstTotp, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

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

        (DestructStep::AwaitSecondTotp, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

        (_, DestructInput::Text(_)) => (
            MessageFlowOutcome::Handled,
            vec![DestructOutput::Text(t!("destruct.verify_fail").to_string())],
        ),

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

pub async fn handle_message_flow(
    bot: &Bot,
    msg: &Message,
    user_id: i64,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id = msg.chat.id;
    let chat_id_str = chat_id.0.to_string();
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

    if !state.is_authorized(user_id).await {
        bot.send_message(chat_id, t!("auth.expired")).await?;
        return Ok(MessageFlowOutcome::Handled);
    }

    let Some(destruct_state) = state.destruct_snapshot(&chat_id_str).await else {
        return Ok(MessageFlowOutcome::NotHandled);
    };

    let action = process_destruct_message(
        msg.text(),
        destruct_state.step,
        state,
        state.self_destruct_key_hash().await.as_deref(),
        None,
    )
    .await;

    match (destruct_state.step, action) {
        (DestructStep::AwaitFirstTotp, DestructMessageAction::ConfirmFirstTotp) => {
            if state
                .confirm_first_destruct_totp(
                    &chat_id_str,
                    msg.text().unwrap().trim(),
                    Instant::now(),
                )
                .await
            {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        t!("destruct.confirm_btn"),
                        "a_destroy_confirm",
                    )],
                    vec![InlineKeyboardButton::callback(
                        t!("destruct.cancelled"),
                        "a_destroy_cancel",
                    )],
                ]);
                bot.send_message(chat_id, t!("destruct.title_2"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
        }
        (DestructStep::AwaitSecondTotp, DestructMessageAction::AwaitingSecondTotp) => {
            match state
                .confirm_second_destruct_totp(
                    &chat_id_str,
                    msg.text().unwrap().trim(),
                    Instant::now(),
                )
                .await
            {
                Err(_) => {
                    bot.send_message(chat_id, t!("destruct.security_warn"))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Ok(true) => {
                    bot.send_message(chat_id, t!("destruct.title_4"))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Ok(false) => {
                    bot.send_message(chat_id, t!("destruct.state_invalid"))
                        .await?;
                }
            }
        }
        (_, DestructMessageAction::VerifyFailed) => {
            bot.send_message(chat_id, t!("destruct.verify_fail"))
                .await?;
        }
        (DestructStep::AwaitSecurityFile, DestructMessageAction::AwaitingFile) => {
            let (file_id, file_name) = if let Some(doc) = msg.document() {
                (Some(doc.file.id.clone()), doc.file_name.clone())
            } else if let Some(photos) = msg.photo() {
                (
                    photos.last().map(|p| p.file.id.clone()),
                    Some(t!("destruct.image_label").to_string()),
                )
            } else {
                (None, None)
            };

            if let Some(fid) = file_id {
                let file = bot.get_file(fid.clone()).await?;
                let mut content = Vec::new();
                bot.download_file(&file.path, &mut content)
                    .await
                    .map_err(std::io::Error::other)?;

                let re_action = process_destruct_message(
                    None,
                    DestructStep::AwaitSecurityFile,
                    state,
                    state.self_destruct_key_hash().await.as_deref(),
                    Some(&content),
                )
                .await;

                match re_action {
                    DestructMessageAction::FileVerified { ref hash_short } => {
                        let file_display = file_name
                            .map(|n| format!("{} | {}", n, hash_short))
                            .unwrap_or_else(|| hash_short.clone());

                        if state
                            .mark_destruct_file_verified(&chat_id_str, Instant::now())
                            .await
                        {
                            let keyboard = InlineKeyboardMarkup::new(vec![
                                vec![InlineKeyboardButton::callback(
                                    t!("destruct.final_btn"),
                                    "a_destroy_final",
                                )],
                                vec![InlineKeyboardButton::callback(
                                    t!("destruct.cancelled"),
                                    "a_destroy_cancel",
                                )],
                            ]);
                            bot.send_message(
                                chat_id,
                                t!("destruct.file_verify_ok", "0" => file_display),
                            )
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard)
                            .await?;
                        }
                    }
                    DestructMessageAction::FileMismatch => {
                        bot.send_message(chat_id, t!("destruct.file_verify_fail"))
                            .await?;
                    }
                    DestructMessageAction::NoSecurityKey => {
                        bot.send_message(chat_id, t!("destruct.no_security_file"))
                            .await?;
                    }
                    _ => {}
                }
            } else {
                bot.send_message(chat_id, t!("destruct.file_send_prompt"))
                    .await?;
            }
        }
        _ => {}
    }
    Ok(MessageFlowOutcome::Handled)
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
    let chat_id_str = chat_id.0.to_string();
    match data {
        "a_destroy_ask" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            state
                .begin_destruct(chat_id_str.clone(), Instant::now())
                .await;
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("destruct.cancelled"),
                "a_destroy_cancel",
            )]]);
            bot.edit_message_text(chat_id, msg_id, t!("destruct.title_1"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_cancel" => {
            if state.cancel_destruct(&chat_id_str).await {
                bot.send_message(chat_id, t!("destruct.cancelled")).await?;
            }
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("destruct.destroy_btn"),
                    "a_destroy_ask",
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
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_confirm" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            if state
                .advance_destruct_step(
                    &chat_id_str,
                    DestructStep::AwaitConfirm,
                    DestructStep::AwaitSecondTotp,
                    Instant::now(),
                )
                .await
            {
                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        t!("destruct.cancelled"),
                        "a_destroy_cancel",
                    )]]);
                bot.edit_message_text(chat_id, msg_id, t!("destruct.title_3"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.state_invalid"))
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_final" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }

            let snapshot = state.destruct_snapshot(&chat_id_str).await;
            if snapshot.map(|s| s.step) == Some(DestructStep::AwaitFinalConfirm) {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.executing"))
                    .await?;
                bot.edit_message_text(chat_id, msg_id, t!("destruct.final_exec"))
                    .parse_mode(ParseMode::Html)
                    .await?;
                let executor = state.self_destruct_executor();
                aegis::core::security::self_destruct::trigger(executor);
                state.cancel_destruct(&chat_id_str).await;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.state_invalid"))
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        _ => Ok(MessageFlowOutcome::NotHandled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use aegis::core::totp::TotpManager;
    use anyhow::Result;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use std::sync::Arc;

    struct MockAdapter;

    #[async_trait]
    impl BotAdapter for MockAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> Result<MessageId> {
            Ok(MessageId("0".to_string()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> Result<()> {
            Ok(())
        }
        async fn delete_message(&self, _target: &TargetId, _msg_id: &MessageId) -> Result<()> {
            Ok(())
        }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn make_test_state(totp_secret: &str) -> AppState {
        AppState::new(
            42,
            TotpManager::new(&SecretString::from(totp_secret.to_string())).unwrap(),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(MockAdapter),
            None,
        )
    }

    #[tokio::test]
    async fn first_totp_valid_returns_confirm() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let totp = state.generate_current_totp().unwrap();
        let action = process_destruct_message(
            Some(&totp),
            DestructStep::AwaitFirstTotp,
            &state,
            None,
            None,
        )
        .await;
        assert_eq!(action, DestructMessageAction::ConfirmFirstTotp);
    }

    #[tokio::test]
    async fn first_totp_invalid_returns_verify_failed() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            Some("000000"),
            DestructStep::AwaitFirstTotp,
            &state,
            None,
            None,
        )
        .await;
        assert_eq!(action, DestructMessageAction::VerifyFailed);
    }

    #[tokio::test]
    async fn first_totp_no_text_returns_noop() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action =
            process_destruct_message(None, DestructStep::AwaitFirstTotp, &state, None, None).await;
        assert_eq!(action, DestructMessageAction::Noop);
    }

    #[tokio::test]
    async fn security_file_match_returns_file_verified() {
        let content = b"test security file content";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None,
            DestructStep::AwaitSecurityFile,
            &state,
            Some(&hash),
            Some(content.as_slice()),
        )
        .await;
        assert!(matches!(action, DestructMessageAction::FileVerified { .. }));
    }

    #[tokio::test]
    async fn security_file_mismatch_returns_mismatch() {
        let content = b"test content";
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None,
            DestructStep::AwaitSecurityFile,
            &state,
            Some(wrong_hash),
            Some(content),
        )
        .await;
        assert_eq!(action, DestructMessageAction::FileMismatch);
    }

    #[tokio::test]
    async fn confirm_step_returns_noop() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action =
            process_destruct_message(None, DestructStep::AwaitConfirm, &state, None, None).await;
        assert_eq!(action, DestructMessageAction::Noop);
    }

    // ── handle_input tests ──

    #[tokio::test]
    async fn handle_input_first_totp_valid_returns_prompt() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let totp = state.generate_current_totp().unwrap();
        let state = Arc::new(state);
        state.record_auth_success(42, Instant::now()).await;
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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
        state.record_auth_success(42, Instant::now()).await;
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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
        state.record_auth_success(42, Instant::now()).await;

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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
        state.record_auth_success(42, Instant::now()).await;
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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
        state.record_auth_success(42, Instant::now()).await;

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
        let content = b"test security file";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let state = Arc::new(AppState::new(
            42,
            TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            Arc::new(TestExecutor),
            Some(hash),
            600,
            Arc::new(MockAdapter),
            None,
        ));
        state.record_auth_success(42, Instant::now()).await;
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
        let state = Arc::new(AppState::new(
            42,
            TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            Arc::new(TestExecutor),
            Some("fake_hash".to_string()),
            600,
            Arc::new(MockAdapter),
            None,
        ));
        state.record_auth_success(42, Instant::now()).await;
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
        let state = Arc::new(make_test_state(&TotpManager::generate_new_secret()).await);
        state.record_auth_success(42, Instant::now()).await;
        state.begin_destruct("chat_cfm".to_string(), Instant::now()).await;
        state.advance_destruct_step("chat_cfm", DestructStep::AwaitFirstTotp, DestructStep::AwaitConfirm, Instant::now()).await;

        let (outcome, outputs) = handle_input(
            &state, "chat_cfm", 42,
            DestructInput::Button(BTN_DESTROY_CONFIRM.to_string()),
            Instant::now(),
        ).await;
        assert_eq!(outcome, MessageFlowOutcome::Handled);
        let snap = state.destruct_snapshot("chat_cfm").await.unwrap();
        assert_eq!(snap.step, DestructStep::AwaitSecondTotp);
    }
}
