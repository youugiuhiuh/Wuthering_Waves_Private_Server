use crate::MAX_INPUT_LENGTH;
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, TimeoutStatus};
use aegis::adapters::common::TargetId;
use aegis::core::xray::config::ConfigManager;
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::prelude::{Message, Requester, ResponseResult};

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, t!("auth.invalid_user")).await?;
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
            t!("message.input_too_long", "0" => MAX_INPUT_LENGTH.to_string()),
        )
        .await?;
        return Ok(());
    }

    let chat_id_str = chat_id.0.to_string();
    match state
        .schedule_timeout_status(&chat_id_str, Duration::from_secs(180))
        .await
    {
        TimeoutStatus::Expired => {
            state.remove_schedule_input(&chat_id_str).await;
            bot.send_message(chat_id, t!("schedule.input_timeout"))
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if msg.text().is_some() || msg.document().is_some() || msg.photo().is_some() {
                bot.send_message(chat_id, t!("schedule.input_prompt"))
                    .await?;
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    match state
        .take_warp_input_status(&chat_id_str, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            bot.send_message(chat_id, t!("message.warp_input_timeout"))
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
                    bot.send_message(chat_id, t!("message.warp_input_empty"))
                        .await?;
                    return Ok(());
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        bot.send_message(chat_id, t!("message.warp_rule_added"))
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            chat_id,
                            t!("message.warp_add_fail", "0" => e.to_string()),
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
            let target = TargetId(chat_id.0.to_string());
            let _ = crate::process_auth_code(&state, &target, user_id, code).await;
            return Ok(());
        }
    }

    Ok(())
}
