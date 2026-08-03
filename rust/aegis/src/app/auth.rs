use std::time::{Duration, Instant};

use anyhow::Result;
use rust_i18n::t;

use crate::app::interaction::{ConversationId, OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use crate::app::state::{AppState, AuthFailureOutcome};

#[allow(clippy::too_many_arguments)]
pub async fn process_auth_code(
    output: &dyn BusinessOutput,
    conversation_id: ConversationId,
    callback_id: Option<String>,
    user_id: i64,
    code: &str,
    state: &AppState,
    max_attempts: u32,
    failure_window: Duration,
    lockout_durations: &[Duration],
) -> Result<bool> {
    if !state.is_admin_user(user_id) {
        output
            .publish(OutputAction::SendText {
                target_conversation: conversation_id.clone(),
                payload: OutputPayload::Text {
                    text: t!("auth.no_permission").to_string(),
                },
                sensitivity: Sensitivity::default(),
            })
            .await?;
        return Ok(false);
    }

    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        output
            .publish(OutputAction::SendText {
                target_conversation: conversation_id.clone(),
                payload: OutputPayload::Text {
                    text: t!("auth.rate_limit", "0" => (remaining.as_secs() / 60).to_string(), "1" => (remaining.as_secs() % 60).to_string()).to_string(),
                },
                sensitivity: Sensitivity::default(),
            })
            .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        let success_text =
            t!("auth.success", "0" => crate::utils::format_duration_human(timeout)).to_string();
        if !state.is_lang_configured().await {
            let lang_text = t!("welcome.select_language").to_string();
            if let Some(cid) = callback_id {
                output
                    .publish(OutputAction::AnswerCallback {
                        callback_id: cid,
                        text: None,
                    })
                    .await?;
            }
            output
                .publish(OutputAction::SendText {
                    target_conversation: conversation_id.clone(),
                    payload: OutputPayload::Text {
                        text: format!("{}\n\n{}", success_text, lang_text),
                    },
                    sensitivity: Sensitivity::default(),
                })
                .await?;
        } else {
            output
                .publish(OutputAction::SendText {
                    target_conversation: conversation_id.clone(),
                    payload: OutputPayload::Text { text: success_text },
                    sensitivity: Sensitivity::default(),
                })
                .await?;
        }
        return Ok(true);
    }

    match state
        .record_auth_failure(
            user_id,
            now,
            max_attempts,
            failure_window,
            lockout_durations,
        )
        .await
    {
        AuthFailureOutcome::Locked { duration } => {
            let duration_str = if duration.as_secs() >= 3600 {
                format!("{} {}", duration.as_secs() / 3600, t!("auth.hours"))
            } else {
                format!("{} {}", duration.as_secs() / 60, t!("auth.minutes"))
            };
            output
                .publish(OutputAction::SendText {
                    target_conversation: conversation_id.clone(),
                    payload: OutputPayload::Text {
                        text: t!("auth.locked", "0" => duration_str).to_string(),
                    },
                    sensitivity: Sensitivity::default(),
                })
                .await?;
        }
        AuthFailureOutcome::Invalid {
            attempts,
            max_attempts,
        } => {
            output
                .publish(OutputAction::SendText {
                    target_conversation: conversation_id.clone(),
                    payload: OutputPayload::Text {
                        text: t!("auth.invalid_code", "0" => attempts.to_string(), "1" => max_attempts.to_string()).to_string(),
                    },
                    sensitivity: Sensitivity::default(),
                })
                .await?;
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interaction::OutputAction;
    use crate::app::output::NoopBotAdapter;
    use crate::common::BotAdapter;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use std::sync::{Arc, Mutex};

    struct FakeOutput {
        pub published: Mutex<Vec<OutputAction>>,
    }

    impl FakeOutput {
        fn new() -> Self {
            Self {
                published: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl BusinessOutput for FakeOutput {
        async fn publish(&self, action: OutputAction) -> Result<()> {
            self.published.lock().unwrap().push(action);
            Ok(())
        }

        fn as_adapter(&self) -> Arc<dyn BotAdapter> {
            NoopBotAdapter::new()
        }
    }

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn process_auth_code_sends_no_permission_message() {
        let output = FakeOutput::new();
        let state = AppState::new(
            Some(999),
            None,
            None,
            std::sync::Arc::new(NoopExecutor),
            None,
            600,
        );
        let conversation_id = ConversationId::new("chat".into()).unwrap();

        let result = process_auth_code(
            &output,
            conversation_id,
            None,
            42,
            "123456",
            &state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap();

        assert!(!result);
        let published = output.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert!(matches!(published[0], OutputAction::SendText { .. }));
    }
}
