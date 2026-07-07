use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use anyhow::Result;
use rust_i18n::t;

use crate::app::state::{AppState, AuthFailureOutcome};

#[allow(clippy::too_many_arguments)]
pub async fn process_auth_code(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    user_id: i64,
    code: &str,
    state: &Arc<AppState>,
    max_attempts: u32,
    failure_window: Duration,
    lockout_durations: &[Duration],
) -> Result<bool> {
    if !state.is_admin_user(user_id) {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("auth.no_permission").to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(false);
    }

    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("auth.rate_limit", "0" => (remaining.as_secs() / 60).to_string(), "1" => (remaining.as_secs() % 60).to_string()).to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        let success_text =
            t!("auth.success", "0" => crate::app::format_duration_human(timeout)).to_string();
        if !state.is_lang_configured().await {
            let lang_text = t!("welcome.select_language").to_string();
            let lang_markup = Markup {
                buttons: vec![vec![
                    InlineButton {
                        text: t!("lang.zh").to_string(),
                        data: "lang:zh".to_string(),
                    },
                    InlineButton {
                        text: t!("lang.en").to_string(),
                        data: "lang:en".to_string(),
                    },
                    InlineButton {
                        text: t!("lang.ja").to_string(),
                        data: "lang:ja".to_string(),
                    },
                ]],
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!("{}\n\n{}", success_text, lang_text),
                        markup: Some(lang_markup),
                    },
                )
                .await?;
        } else {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: success_text,
                        markup: None,
                    },
                )
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
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("auth.locked", "0" => duration_str).to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        AuthFailureOutcome::Invalid {
            attempts,
            max_attempts,
        } => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("auth.invalid_code", "0" => attempts.to_string(), "1" => max_attempts.to_string()).to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
    }

    Ok(false)
}
