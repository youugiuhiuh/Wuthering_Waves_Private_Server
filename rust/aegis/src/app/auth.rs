use std::sync::Arc;
use std::time::{Duration, Instant};

use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use anyhow::Result;

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
                    text: "❌ 无权操作".to_string(),
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
                    text: format!(
                        "⚠️ 尝试过于频繁，请稍后再试。冷却剩余约 {} 分 {} 秒。",
                        remaining.as_secs() / 60,
                        remaining.as_secs() % 60
                    ),
                    markup: None,
                },
            )
            .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        adapter
            .send_message(
                target,
                MessageContent {
                    text: format!(
                        "✅ 认证成功！会话有效期 {}。",
                        crate::utils::format_duration_human(timeout)
                    ),
                    markup: None,
                },
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
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!(
                            "❌ 验证失败次数过多，已进入冷却。\n⏱️ 锁定时间: {}\n⚠️ 请稍后再试。",
                            duration_str
                        ),
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
                        text: format!(
                            "❌ TOTP 验证码无效，请检查后重试。（已失败 {} 次 / {} 次）",
                            attempts, max_attempts
                        ),
                        markup: None,
                    },
                )
                .await?;
        }
    }

    Ok(false)
}
