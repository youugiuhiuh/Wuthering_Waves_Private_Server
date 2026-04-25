use std::sync::Arc;
use std::time::{Duration, Instant};

use teloxide::prelude::*;

use crate::app::state::{AppState, AuthFailureOutcome};

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
    if !state.is_admin_user(user_id) {
        bot.send_message(chat_id, "❌ 无权操作").await?;
        return Ok(false);
    }

    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        bot.send_message(
            chat_id,
            format!(
                "⚠️ 尝试过于频繁，请稍后再试。冷却剩余约 {} 分 {} 秒。",
                remaining.as_secs() / 60,
                remaining.as_secs() % 60
            ),
        )
        .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        bot.send_message(
            chat_id,
            format!(
                "✅ 认证成功！会话有效期 {}。请使用 /menu 开始管理。",
                crate::format_duration_human(timeout)
            ),
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
                format!("{} 小时", duration.as_secs() / 3600)
            } else {
                format!("{} 分钟", duration.as_secs() / 60)
            };
            bot.send_message(
                chat_id,
                format!(
                    "❌ 验证失败次数过多，已进入冷却。\n⏱️ 锁定时间: {}\n⚠️ 请稍后再试。",
                    duration_str
                ),
            )
            .await?;
        }
        AuthFailureOutcome::Invalid {
            attempts,
            max_attempts,
        } => {
            bot.send_message(
                chat_id,
                format!(
                    "❌ TOTP 验证码无效，请检查后重试。（已失败 {} 次 / {} 次）",
                    attempts, max_attempts
                ),
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
