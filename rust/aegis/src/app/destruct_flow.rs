use std::time::{Duration, Instant};

use rust_i18n::t;
use sha2::Digest;

use crate::adapters::common::{InlineButton, Markup, MessageContent, MessageId};
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

/// Process destruct message flow using BotAdapter instead of teloxide types.
/// Returns (MessageFlowOutcome, Option<MessageContent>) where MessageContent is the response to send.
pub async fn handle_message_flow_adapter(
    text: Option<&str>,
    file_content: Option<&[u8]>,
    state: &AppState,
    chat_id: &str,
    user_id: i64,
) -> (
    MessageFlowOutcome,
    Option<(MessageContent, Option<MessageId>)>,
) {
    match state
        .touch_destruct(chat_id, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(chat_id).await;
            return (
                MessageFlowOutcome::Handled,
                Some((
                    MessageContent {
                        text: t!("destruct.timeout").to_string(),
                        markup: None,
                    },
                    None,
                )),
            );
        }
        TimeoutStatus::NotTracked => return (MessageFlowOutcome::NotHandled, None),
        TimeoutStatus::Active => {}
    }

    if !state.is_admin_user(user_id) && !state.is_authorized(user_id).await {
        return (
            MessageFlowOutcome::Handled,
            Some((
                MessageContent {
                    text: t!("auth.expired").to_string(),
                    markup: None,
                },
                None,
            )),
        );
    }

    let Some(destruct_state) = state.destruct_snapshot(chat_id).await else {
        return (MessageFlowOutcome::NotHandled, None);
    };

    let action = process_destruct_message(
        text,
        destruct_state.step,
        state,
        state.self_destruct_key_hash().await.as_deref(),
        file_content,
    )
    .await;

    match (destruct_state.step, action) {
        (DestructStep::AwaitFirstTotp, DestructMessageAction::ConfirmFirstTotp) => {
            if state
                .confirm_first_destruct_totp(chat_id, text.unwrap().trim(), Instant::now())
                .await
            {
                let markup = Markup {
                    buttons: vec![
                        vec![InlineButton {
                            text: t!("destruct.confirm_btn").to_string(),
                            data: "a_destroy_confirm".to_string(),
                        }],
                        vec![InlineButton {
                            text: t!("destruct.cancelled").to_string(),
                            data: "a_destroy_cancel".to_string(),
                        }],
                    ],
                };
                (
                    MessageFlowOutcome::Handled,
                    Some((
                        MessageContent {
                            text: t!("destruct.title_2").to_string(),
                            markup: Some(markup),
                        },
                        None,
                    )),
                )
            } else {
                (MessageFlowOutcome::Handled, None)
            }
        }
        (DestructStep::AwaitSecondTotp, DestructMessageAction::AwaitingSecondTotp) => {
            match state
                .confirm_second_destruct_totp(chat_id, text.unwrap().trim(), Instant::now())
                .await
            {
                Err(_) => (
                    MessageFlowOutcome::Handled,
                    Some((
                        MessageContent {
                            text: t!("destruct.security_warn").to_string(),
                            markup: None,
                        },
                        None,
                    )),
                ),
                Ok(true) => (
                    MessageFlowOutcome::Handled,
                    Some((
                        MessageContent {
                            text: t!("destruct.title_4").to_string(),
                            markup: None,
                        },
                        None,
                    )),
                ),
                Ok(false) => (
                    MessageFlowOutcome::Handled,
                    Some((
                        MessageContent {
                            text: t!("destruct.state_invalid").to_string(),
                            markup: None,
                        },
                        None,
                    )),
                ),
            }
        }
        (_, DestructMessageAction::VerifyFailed) => (
            MessageFlowOutcome::Handled,
            Some((
                MessageContent {
                    text: t!("destruct.verify_fail").to_string(),
                    markup: None,
                },
                None,
            )),
        ),
        (DestructStep::AwaitSecurityFile, DestructMessageAction::AwaitingFile) => {
            if file_content.is_some() {
                let re_action = process_destruct_message(
                    None,
                    DestructStep::AwaitSecurityFile,
                    state,
                    state.self_destruct_key_hash().await.as_deref(),
                    file_content,
                )
                .await;
                match re_action {
                    DestructMessageAction::FileVerified { ref hash_short } => {
                        if state
                            .mark_destruct_file_verified(chat_id, Instant::now())
                            .await
                        {
                            let markup = Markup {
                                buttons: vec![
                                    vec![InlineButton {
                                        text: t!("destruct.final_btn").to_string(),
                                        data: "a_destroy_final".to_string(),
                                    }],
                                    vec![InlineButton {
                                        text: t!("destruct.cancelled").to_string(),
                                        data: "a_destroy_cancel".to_string(),
                                    }],
                                ],
                            };
                            (
                                MessageFlowOutcome::Handled,
                                Some((
                                    MessageContent {
                                        text: t!("destruct.file_verify_ok", "0" => hash_short)
                                            .to_string(),
                                        markup: Some(markup),
                                    },
                                    None,
                                )),
                            )
                        } else {
                            (MessageFlowOutcome::Handled, None)
                        }
                    }
                    DestructMessageAction::FileMismatch => (
                        MessageFlowOutcome::Handled,
                        Some((
                            MessageContent {
                                text: t!("destruct.file_verify_fail").to_string(),
                                markup: None,
                            },
                            None,
                        )),
                    ),
                    DestructMessageAction::NoSecurityKey => (
                        MessageFlowOutcome::Handled,
                        Some((
                            MessageContent {
                                text: t!("destruct.no_security_file").to_string(),
                                markup: None,
                            },
                            None,
                        )),
                    ),
                    _ => (MessageFlowOutcome::Handled, None),
                }
            } else {
                (
                    MessageFlowOutcome::Handled,
                    Some((
                        MessageContent {
                            text: t!("destruct.file_send_prompt").to_string(),
                            markup: None,
                        },
                        None,
                    )),
                )
            }
        }
        _ => (MessageFlowOutcome::Handled, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use anyhow::Result;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use sha2::Digest;
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
}
