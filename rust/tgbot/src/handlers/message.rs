use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::prelude::{Message, Requester, ResponseResult};
use tgbot::logic::config::ConfigManager;
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, TimeoutStatus};
use crate::MAX_INPUT_LENGTH;

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, "⚠️ 无法识别用户身份，请访问管理员检查权限")
            .await?;
        return Ok(());
    };
    let user_id = from.id.0 as i64;

    if !state.is_admin_user(user_id) {
        return Ok(());
    }

    if let Some(text) = msg.text()
        && text.len() > MAX_INPUT_LENGTH
    {
        bot.send_message(
            chat_id,
            format!("⚠️ 输入过长，请控制在 {} 字符以内。", MAX_INPUT_LENGTH),
        )
        .await?;
        return Ok(());
    }

    match state
        .schedule_timeout_status(chat_id, Duration::from_secs(180))
        .await
    {
        TimeoutStatus::Expired => {
            state.remove_schedule_input(chat_id).await;
            bot.send_message(chat_id, "⏳ 定时任务选择超时 (180s)，已自动取消。")
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if msg.text().is_some() || msg.document().is_some() || msg.photo().is_some() {
                bot.send_message(
                    chat_id,
                    "ℹ️ 请通过面板按钮选择 星期/小时/分钟，然后点击“确认创建任务”。",
                )
                .await?;
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    match state
        .take_warp_input_status(chat_id, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            bot.send_message(chat_id, "⏳ 输入超时 (60s)，已自动取消。")
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if let Some(text) = msg.text() {
                let rules: Vec<String> = text
                    .split([',', '，', '\n'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if rules.is_empty() {
                    bot.send_message(chat_id, "⚠️ 输入为空，请重新输入或使用 /menu 返回。")
                        .await?;
                    return Ok(());
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        bot.send_message(chat_id, "✅ WARP 分流规则已添加并重载核心。")
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ 添加失败: {}", e))
                            .await?;
                    }
                }
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    if destruct_flow::handle_message_flow(&bot, &msg, user_id, &state).await?
        == MessageFlowOutcome::Handled
    {
        return Ok(());
    }

    if let Some(text) = msg.text() {
        let code = text.trim();
        if crate::looks_like_totp_code(code) && !state.is_authorized(user_id).await {
            let _ = crate::process_auth_code(&bot, chat_id, user_id, code, &state).await?;
            return Ok(());
        }
    }

    Ok(())
}