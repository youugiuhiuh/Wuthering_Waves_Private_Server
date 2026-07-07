use super::subscription;
use crate::MAX_INPUT_LENGTH;
use aegis::adapters::common::TargetId;
use aegis::app::destruct_flow;
use aegis::app::destruct_flow::MessageFlowOutcome;
use aegis::app::state::{AppState, DestructStep, TimeoutStatus};
use aegis::core::xray::config::ConfigManager;
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::net::Download;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Message, Requester, ResponseResult};
use teloxide::types::ParseMode;

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, t!("auth.invalid_user"))
            .parse_mode(ParseMode::Html)
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
            t!("message.input_too_long", "0" => MAX_INPUT_LENGTH.to_string()),
        )
        .parse_mode(ParseMode::Html)
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
                .parse_mode(ParseMode::Html)
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if msg.text().is_some() || msg.document().is_some() || msg.photo().is_some() {
                bot.send_message(chat_id, t!("schedule.input_prompt"))
                    .parse_mode(ParseMode::Html)
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
                .parse_mode(ParseMode::Html)
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
                        .parse_mode(ParseMode::Html)
                        .await?;
                    return Ok(());
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        bot.send_message(chat_id, t!("message.warp_rule_added"))
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            chat_id,
                            t!("message.warp_add_fail", "0" => e.to_string()),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    }
                }
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    if let Some(text) = msg.text()
        && subscription::handle_text_input(&bot, msg.chat.id, &state, text).await?
    {
        return Ok(());
    }

    let file_content: Option<Vec<u8>> = match state.destruct_snapshot(&chat_id_str).await {
        Some(s) if s.step == DestructStep::AwaitSecurityFile => {
            let file_id = msg.document().map(|d| d.file.id.clone()).or_else(|| {
                msg.photo()
                    .and_then(|p| p.last().map(|ph| ph.file.id.clone()))
            });
            match file_id {
                Some(fid) => match bot.get_file(&fid).await {
                    Ok(file) => {
                        let mut content = Vec::new();
                        match bot.download_file(&file.path, &mut content).await {
                            Ok(()) => Some(content),
                            Err(_) => None,
                        }
                    }
                    Err(_) => None,
                },
                None => None,
            }
        }
        _ => None,
    };

    let (outcome, response) = destruct_flow::handle_message_flow_adapter(
        msg.text(),
        file_content.as_deref(),
        &state,
        &chat_id_str,
        user_id,
    )
    .await;
    if outcome == MessageFlowOutcome::Handled {
        if let Some((content, _)) = response {
            let mut msg_r = bot
                .send_message(chat_id, content.text)
                .parse_mode(ParseMode::Html);
            if let Some(ref markup) = content.markup {
                let rows: Vec<Vec<teloxide::types::InlineKeyboardButton>> = markup
                    .buttons
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|btn| {
                                teloxide::types::InlineKeyboardButton::callback(
                                    &btn.text, &btn.data,
                                )
                            })
                            .collect()
                    })
                    .collect();
                msg_r = msg_r.reply_markup(teloxide::types::InlineKeyboardMarkup::new(rows));
            }
            msg_r.await?;
        }
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
