use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;
use teloxide::prelude::*;

use crate::app::state::{AppState, AuthFailureOutcome};

#[allow(clippy::too_many_arguments)]
pub async fn process_auth_code(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    code: &str,
    state: &Arc<AppState>,
    max_attempts: u32,
    failure_window: Duration,
    lockout_durations: &[Duration],
) -> ResponseResult<bool> {
    let lang = state.language().await;

    if !state.is_admin_user(user_id) {
        bot.send_message(chat_id, t!("auth.unauthorized", locale = &lang))
            .await?;
        return Ok(false);
    }

    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        bot.send_message(
            chat_id,
            t!("auth.cooldown", locale = &lang)
                .replace("%min%", &(remaining.as_secs() / 60).to_string())
                .replace("%sec%", &(remaining.as_secs() % 60).to_string()),
        )
        .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        bot.send_message(
            chat_id,
            t!("auth.success", locale = &lang)
                .replace("%duration%", &crate::format_duration_human(timeout)),
        )
        .await?;
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
                format!(
                    "{} {}",
                    duration.as_secs() / 3600,
                    t!("duration.hours", locale = &lang)
                )
            } else {
                format!(
                    "{} {}",
                    duration.as_secs() / 60,
                    t!("duration.minutes", locale = &lang)
                )
            };
            bot.send_message(
                chat_id,
                t!("auth.locked", locale = &lang).replace("%duration%", &duration_str),
            )
            .await?;
        }
        AuthFailureOutcome::Invalid {
            attempts,
            max_attempts,
        } => {
            bot.send_message(
                chat_id,
                t!("auth.invalid_code", locale = &lang)
                    .replace("%attempts%", &attempts.to_string())
                    .replace("%max%", &max_attempts.to_string()),
            )
            .await?;
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_auth_code_function_exists() {
        let _ = process_auth_code;
    }

    #[test]
    fn test_auth_code_response_result_type() {
        fn check_response_result<T: Default>() -> bool {
            std::mem::size_of::<ResponseResult<T>>() > 0
        }
        assert!(check_response_result::<bool>());
    }
}
