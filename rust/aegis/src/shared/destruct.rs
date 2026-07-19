use std::time::Instant;

use anyhow::Result;
use rust_i18n::t;
use sha2::Digest;

use aegis::adapters::common::{DestructKey, InlineButton, Markup, MessageContent};
use aegis::shared::types::{CallbackEvent, MessageEvent};

use crate::app::state::{AppState, DestructStatus};

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

/// Outcome of a flow interception in the shared (platform-agnostic) layer.
/// Distinct from the legacy `MessageFlowOutcome` in `app::destruct_flow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowOutcome {
    Handled,
    NotHandled,
}

/// Pure logic layer: given the current destruct step and user input,
/// decide what action should be taken.
/// Does NOT depend on teloxide Bot/Message types — directly testable.
pub async fn process_destruct_message(
    text: Option<&str>,
    step: DestructStatus,
    state: &AppState,
    self_destruct_key_hash: Option<&str>,
    file_content: Option<&[u8]>,
) -> DestructMessageAction {
    match step {
        DestructStatus::AwaitFirstTotp => match text {
            Some(code) if state.verify_totp(code.trim()) => DestructMessageAction::ConfirmFirstTotp,
            Some(_) => DestructMessageAction::VerifyFailed,
            None => DestructMessageAction::Noop,
        },
        DestructStatus::AwaitSecondTotp => match text {
            Some(code) if state.verify_totp(code.trim()) => {
                DestructMessageAction::AwaitingSecondTotp
            }
            Some(_) => DestructMessageAction::VerifyFailed,
            None => DestructMessageAction::Noop,
        },
        DestructStatus::AwaitSecurityFile => {
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
        DestructStatus::AwaitFirstConfirm | DestructStatus::AwaitFinalConfirm => {
            DestructMessageAction::Noop
        }
        _ => DestructMessageAction::Noop,
    }
}

async fn authorize_flow(state: &AppState, key: &DestructKey, now: Instant) -> Result<()> {
    if state.self_destruct_key_hash().await.is_none() {
        anyhow::bail!("self-destruct security file is not configured");
    }
    if !state.is_admin_user(&key.principal) || !state.is_authorized(&key.principal).await {
        anyhow::bail!("self-destruct authorization required");
    }
    state
        .ensure_destruct_active(key, now)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

fn btn(text: &str, data: &str) -> InlineButton {
    InlineButton {
        text: text.to_string(),
        data: data.to_string(),
    }
}

#[allow(dead_code)]
/// Port of `app::destruct_flow::handle_message_flow` onto the unified `BotAdapter`.
/// Returns `FlowOutcome` describing whether the message was consumed by the
/// destruct flow.
pub async fn intercept_message(msg: &MessageEvent, state: &AppState) -> Result<FlowOutcome> {
    let adapter = msg.adapter.as_ref();
    let target = &msg.target;
    let key = DestructKey {
        principal: msg.principal.clone(),
        target: msg.target.clone(),
    };
    let now = Instant::now();

    if authorize_flow(state, &key, now).await.is_err() {
        return Ok(FlowOutcome::NotHandled);
    }

    let Some(destruct_state) = state.destruct_snapshot(&key).await else {
        return Ok(FlowOutcome::NotHandled);
    };

    let action = process_destruct_message(
        msg.text.as_deref(),
        destruct_state.status,
        state,
        state.self_destruct_key_hash().await.as_deref(),
        None,
    )
    .await;

    match (destruct_state.status, action) {
        (DestructStatus::AwaitFirstTotp, DestructMessageAction::ConfirmFirstTotp) => {
            if state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitFirstTotp,
                    DestructStatus::AwaitFirstConfirm,
                )
                .await
            {
                let keyboard = Markup {
                    buttons: vec![
                        vec![btn(
                            t!("destruct.confirm_btn").as_ref(),
                            "a_destroy_confirm",
                        )],
                        vec![btn(t!("destruct.cancelled").as_ref(), "a_destroy_cancel")],
                    ],
                };
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.title_2").into(),
                            markup: Some(keyboard),
                        },
                    )
                    .await?;
            }
        }
        (DestructStatus::AwaitSecondTotp, DestructMessageAction::AwaitingSecondTotp) => {
            if state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitSecondTotp,
                    DestructStatus::AwaitSecurityFile,
                )
                .await
            {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.title_4").into(),
                            markup: None,
                        },
                    )
                    .await?;
            } else {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.state_invalid").into(),
                            markup: None,
                        },
                    )
                    .await?;
            }
        }
        (_, DestructMessageAction::VerifyFailed) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("destruct.verify_fail").into(),
                        markup: None,
                    },
                )
                .await?;
        }
        (DestructStatus::AwaitSecurityFile, DestructMessageAction::AwaitingFile) => {
            if let Some(fid) = msg.file_id.as_ref() {
                let content = adapter.download_file(fid).await?;

                let re_action = process_destruct_message(
                    None,
                    DestructStatus::AwaitSecurityFile,
                    state,
                    state.self_destruct_key_hash().await.as_deref(),
                    Some(&content),
                )
                .await;

                match re_action {
                    DestructMessageAction::FileVerified { ref hash_short } => {
                        let file_display = msg
                            .file_name
                            .as_ref()
                            .map(|n| format!("{} | {}", n, hash_short))
                            .unwrap_or_else(|| hash_short.clone());
                        if state
                            .advance_destruct_step(
                                &key,
                                DestructStatus::AwaitSecurityFile,
                                DestructStatus::AwaitFinalConfirm,
                            )
                            .await
                        {
                            let keyboard = Markup {
                                buttons: vec![
                                    vec![btn(t!("destruct.final_btn").as_ref(), "a_destroy_final")],
                                    vec![btn(
                                        t!("destruct.cancelled").as_ref(),
                                        "a_destroy_cancel",
                                    )],
                                ],
                            };
                            adapter
                                .send_message(
                                    target,
                                    MessageContent {
                                        text: t!("destruct.file_verify_ok", "0" => file_display)
                                            .into(),
                                        markup: Some(keyboard),
                                    },
                                )
                                .await?;
                        }
                    }
                    DestructMessageAction::FileMismatch => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.file_verify_fail").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                    DestructMessageAction::NoSecurityKey => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.no_security_file").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                    _ => {}
                }
            } else {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.file_send_prompt").into(),
                            markup: None,
                        },
                    )
                    .await?;
            }
        }
        (DestructStatus::AwaitFirstConfirm, _) => {
            if let Some(ref text) = msg.text {
                let t = text.trim().to_lowercase();
                if t == "confirm" || t == "確認" || t == "yes" || state.verify_totp(text.trim()) {
                    if state
                        .advance_destruct_step(
                            &key,
                            DestructStatus::AwaitFirstConfirm,
                            DestructStatus::AwaitFinalConfirm,
                        )
                        .await
                    {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.title_4").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                    } else {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.state_invalid").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                } else if t == "cancel" || t == "取消" || t == "no" {
                    state.cancel_destruct(&key).await;
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("destruct.cancelled").into(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
        }
        (DestructStatus::AwaitFinalConfirm, _) => {
            if let Some(ref text) = msg.text {
                let t = text.trim().to_lowercase();
                if t == "confirm" || t == "確認" || t == "yes" || state.verify_totp(text.trim()) {
                    let executor = state.self_destruct_executor();
                    if state.claim_execution(&key, None, now).await {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.final_exec").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                        let success =
                            aegis::core::security::self_destruct::execute_supervised(executor)
                                .await
                                .is_ok();
                        state.complete_execution(&key, success).await;
                    } else {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("destruct.state_invalid").into(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                } else if t == "cancel" || t == "取消" || t == "no" {
                    state.cancel_destruct(&key).await;
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("destruct.cancelled").into(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
        }
        _ => {}
    }
    Ok(FlowOutcome::Handled)
}

async fn callback_timeout(cb: &CallbackEvent, state: &AppState) -> Result<FlowOutcome> {
    let adapter = cb.adapter.as_ref();
    let target = &cb.target;
    let key = DestructKey {
        principal: cb.principal.clone(),
        target: cb.target.clone(),
    };
    let snapshot = state.destruct_snapshot(&key).await;
    match snapshot {
        Some(s) if s.deadline <= Instant::now() => {
            state.cancel_destruct(&key).await;
            adapter
                .answer_callback(
                    target,
                    &cb.callback_id,
                    Some(t!("destruct.callback_timeout").to_string()),
                )
                .await?;
            adapter
                .edit_message(
                    target,
                    &cb.msg_id,
                    MessageContent {
                        text: t!("destruct.timeout").into(),
                        markup: None,
                    },
                )
                .await?;
            Ok(FlowOutcome::Handled)
        }
        Some(_) => Ok(FlowOutcome::NotHandled),
        None => Ok(FlowOutcome::NotHandled),
    }
}

async fn callback_action(cb: &CallbackEvent, state: &AppState) -> Result<FlowOutcome> {
    let adapter = cb.adapter.as_ref();
    let target = &cb.target;
    let key = DestructKey {
        principal: cb.principal.clone(),
        target: cb.target.clone(),
    };
    match cb.data.as_str() {
        "a_destroy_ask" => {
            if !state.is_authorized(&cb.principal).await {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("auth.expired").to_string()),
                    )
                    .await?;
                return Ok(FlowOutcome::Handled);
            }
            let _ = state.begin_destruct(&key, Instant::now()).await;
            let keyboard = Markup {
                buttons: vec![vec![btn(
                    t!("destruct.cancelled").as_ref(),
                    "a_destroy_cancel",
                )]],
            };
            adapter
                .edit_message(
                    target,
                    &cb.msg_id,
                    MessageContent {
                        text: t!("destruct.title_1").into(),
                        markup: Some(keyboard),
                    },
                )
                .await?;
            Ok(FlowOutcome::Handled)
        }
        "a_destroy_cancel" => {
            if state.cancel_destruct(&key).await {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("destruct.cancelled").into(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            let keyboard = Markup {
                buttons: vec![
                    vec![btn(t!("destruct.destroy_btn").as_ref(), "a_destroy_ask")],
                    vec![btn(t!("menu.back_settings").as_ref(), "m_settings")],
                ],
            };
            adapter
                .edit_message(
                    target,
                    &cb.msg_id,
                    MessageContent {
                        text: format!(
                            "{}\n\n{}",
                            t!("menu.danger_zone"),
                            t!("menu.danger_zone_desc")
                        ),
                        markup: Some(keyboard),
                    },
                )
                .await?;
            Ok(FlowOutcome::Handled)
        }
        "a_destroy_confirm" => {
            if authorize_flow(state, &key, Instant::now()).await.is_err() {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("auth.expired").to_string()),
                    )
                    .await?;
                return Ok(FlowOutcome::Handled);
            }
            if state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitFirstConfirm,
                    DestructStatus::AwaitSecondTotp,
                )
                .await
            {
                let keyboard = Markup {
                    buttons: vec![vec![btn(
                        t!("destruct.cancelled").as_ref(),
                        "a_destroy_cancel",
                    )]],
                };
                adapter
                    .edit_message(
                        target,
                        &cb.msg_id,
                        MessageContent {
                            text: t!("destruct.title_3").into(),
                            markup: Some(keyboard),
                        },
                    )
                    .await?;
            } else {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("destruct.state_invalid").to_string()),
                    )
                    .await?;
            }
            Ok(FlowOutcome::Handled)
        }
        "a_destroy_final" => {
            if authorize_flow(state, &key, Instant::now()).await.is_err() {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("auth.expired").to_string()),
                    )
                    .await?;
                return Ok(FlowOutcome::Handled);
            }

            let snapshot = state.destruct_snapshot(&key).await;
            if snapshot.map(|s| s.status) == Some(DestructStatus::AwaitFinalConfirm) {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("destruct.executing").to_string()),
                    )
                    .await?;
                adapter
                    .edit_message(
                        target,
                        &cb.msg_id,
                        MessageContent {
                            text: t!("destruct.final_exec").into(),
                            markup: None,
                        },
                    )
                    .await?;
                if state.claim_execution(&key, None, Instant::now()).await {
                    let executor = state.self_destruct_executor();
                    let success =
                        aegis::core::security::self_destruct::execute_supervised(executor)
                            .await
                            .is_ok();
                    state.complete_execution(&key, success).await;
                }
            } else {
                adapter
                    .answer_callback(
                        target,
                        &cb.callback_id,
                        Some(t!("destruct.state_invalid").to_string()),
                    )
                    .await?;
            }
            Ok(FlowOutcome::Handled)
        }
        _ => Ok(FlowOutcome::NotHandled),
    }
}

#[allow(dead_code)]
/// Port of the `handle_callback_timeout` + `handle_callback_action` sequence
/// (as invoked from the telegram callback handler) onto the unified
/// `BotAdapter`. Timeout is checked first, then the action data.
pub async fn intercept_callback(cb: &CallbackEvent, state: &AppState) -> Result<FlowOutcome> {
    if callback_timeout(cb, state).await? == FlowOutcome::Handled {
        return Ok(FlowOutcome::Handled);
    }
    callback_action(cb, state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::adapters::common::{
        BotAdapter, DestructKey, MessageContent, MessageId, Platform, Principal, TargetId,
    };
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use aegis::core::totp::TotpManager;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use std::sync::Arc;
    use std::time::Instant;

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

    fn test_key() -> DestructKey {
        DestructKey {
            principal: Principal::telegram(42),
            target: TargetId("42".into()),
        }
    }

    async fn make_test_state(totp_secret: &str) -> AppState {
        let state = AppState::new(
            42,
            None,
            TotpManager::new(&SecretString::from(totp_secret.to_string())).unwrap(),
            Arc::new(TestExecutor),
            Some("test-hash".to_string()),
            600,
            Arc::new(MockAdapter),
        );
        state
            .record_auth_success(&Principal::telegram(42), Instant::now())
            .await;
        state
    }

    #[tokio::test]
    async fn first_totp_valid_returns_confirm() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let totp = state.generate_current_totp().unwrap();
        let action = process_destruct_message(
            Some(&totp),
            DestructStatus::AwaitFirstTotp,
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
            DestructStatus::AwaitFirstTotp,
            &state,
            None,
            None,
        )
        .await;
        assert_eq!(action, DestructMessageAction::VerifyFailed);
    }

    #[tokio::test]
    async fn security_file_match_returns_file_verified() {
        let content = b"test security file content";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let action = process_destruct_message(
            None,
            DestructStatus::AwaitSecurityFile,
            &state,
            Some(&hash),
            Some(content.as_slice()),
        )
        .await;
        assert!(matches!(action, DestructMessageAction::FileVerified { .. }));
    }

    fn callback_event(data: &str) -> CallbackEvent {
        CallbackEvent {
            adapter: Arc::new(MockAdapter),
            target: TargetId("42".into()),
            principal: Principal::telegram(42),
            msg_id: MessageId("0".into()),
            data: data.into(),
            callback_id: "q1".into(),
            session_timeout_secs: 600,
        }
    }

    #[tokio::test]
    async fn intercept_callback_ask_begins_destruct() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let outcome = intercept_callback(&callback_event("a_destroy_ask"), &state)
            .await
            .unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        assert!(state.destruct_snapshot(&test_key()).await.is_some());
    }

    #[tokio::test]
    async fn intercept_callback_cancel_cancels_destruct() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        state
            .begin_destruct(&test_key(), Instant::now())
            .await
            .unwrap();
        let outcome = intercept_callback(&callback_event("a_destroy_cancel"), &state)
            .await
            .unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        assert!(state.destruct_snapshot(&test_key()).await.is_none());
    }

    #[tokio::test]
    async fn intercept_message_totp_advances_step() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        state
            .begin_destruct(&test_key(), Instant::now())
            .await
            .unwrap();
        let totp = state.generate_current_totp().unwrap();
        let msg = MessageEvent {
            adapter: Arc::new(MockAdapter),
            target: TargetId("42".into()),
            principal: Principal::telegram(42),
            text: Some(totp),
            file_id: None,
            file_name: None,
            attachment: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        let snap = state.destruct_snapshot(&test_key()).await.unwrap();
        assert_eq!(snap.status, DestructStatus::AwaitFirstConfirm);
    }

    #[tokio::test]
    async fn confirm_text_triggers_destruct_at_final_confirm() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        state
            .begin_destruct(&test_key(), Instant::now())
            .await
            .unwrap();
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstTotp,
                DestructStatus::AwaitFirstConfirm,
            )
            .await;
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstConfirm,
                DestructStatus::AwaitFinalConfirm,
            )
            .await;
        let totp = state.generate_current_totp().unwrap();
        let msg = MessageEvent {
            adapter: Arc::new(MockAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            principal: Principal::telegram(42),
            text: Some(totp),
            file_id: None,
            file_name: None,
            attachment: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        let snap = state.destruct_snapshot(&test_key()).await.unwrap();
        assert_eq!(snap.status, DestructStatus::Succeeded);
    }

    #[tokio::test]
    async fn confirm_text_advances_await_confirm_to_final() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        state
            .begin_destruct(&test_key(), Instant::now())
            .await
            .unwrap();
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstTotp,
                DestructStatus::AwaitFirstConfirm,
            )
            .await;
        let msg = MessageEvent {
            adapter: Arc::new(MockAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            principal: Principal::telegram(42),
            text: Some("confirm".into()),
            file_id: None,
            file_name: None,
            attachment: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        let snap = state.destruct_snapshot(&test_key()).await.unwrap();
        assert_eq!(snap.status, DestructStatus::AwaitFinalConfirm);
    }

    #[tokio::test]
    async fn confirm_text_cancels_destruct_on_no() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        state
            .begin_destruct(&test_key(), Instant::now())
            .await
            .unwrap();
        state
            .advance_destruct_step(
                &test_key(),
                DestructStatus::AwaitFirstTotp,
                DestructStatus::AwaitFirstConfirm,
            )
            .await;
        let msg = MessageEvent {
            adapter: Arc::new(MockAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            principal: Principal::telegram(42),
            text: Some("cancel".into()),
            file_id: None,
            file_name: None,
            attachment: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        assert!(state.destruct_snapshot(&test_key()).await.is_none());
    }

    #[tokio::test]
    async fn intercept_message_no_destruct_returns_not_handled() {
        let secret = TotpManager::generate_new_secret();
        let state = make_test_state(&secret).await;
        let msg = MessageEvent {
            adapter: Arc::new(MockAdapter),
            target: TargetId("99".into()),
            principal: Principal::telegram(42),
            text: Some("hi".into()),
            file_id: None,
            file_name: None,
            attachment: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::NotHandled);
    }
}
