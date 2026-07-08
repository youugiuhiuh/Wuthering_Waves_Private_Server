use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;
use sha2::Digest;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

use crate::app::state::{AppState, DestructStep, TimeoutStatus};

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
        async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> aegis::adapters::common::PlatformCapabilities {
            aegis::adapters::common::PlatformCapabilities::TELEGRAM
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
}
