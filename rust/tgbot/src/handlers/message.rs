use crate::MAX_INPUT_LENGTH;
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, TimeoutStatus};
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::prelude::{Message, Requester, ResponseResult};
use tgbot::logic::config::ConfigManager;

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = state.language().await;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, t!("auth.no_identity", locale = &lang))
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
            t!("message.input_too_long", locale = &lang)
                .replace("%max%", &MAX_INPUT_LENGTH.to_string()),
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
            bot.send_message(chat_id, t!("message.schedule_timeout", locale = &lang))
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if msg.text().is_some() || msg.document().is_some() || msg.photo().is_some() {
                bot.send_message(chat_id, t!("message.schedule_active", locale = &lang))
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
            bot.send_message(chat_id, t!("message.warp_timeout", locale = &lang))
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
                    bot.send_message(chat_id, t!("message.warp_input_empty", locale = &lang))
                        .await?;
                    return Ok(());
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        bot.send_message(chat_id, t!("message.warp_rules_added", locale = &lang))
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            chat_id,
                            t!("message.warp_add_failed", locale = &lang)
                                .replace("%error%", &e.to_string()),
                        )
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
