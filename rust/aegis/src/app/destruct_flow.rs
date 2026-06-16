use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;
use sha2::{Digest, Sha256};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

use crate::app::state::{AppState, DestructStep, TimeoutStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageFlowOutcome {
    Handled,
    NotHandled,
}

pub async fn handle_message_flow(
    bot: &Bot,
    msg: &Message,
    user_id: i64,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id = msg.chat.id;
    let chat_id_str = chat_id.0.to_string();
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.send_message(chat_id, t!("destruct.timeout")).await?;
            return Ok(MessageFlowOutcome::Handled);
        }
        TimeoutStatus::NotTracked => return Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::Active => {}
    }

    if !state.is_authorized(user_id).await {
        bot.send_message(chat_id, t!("auth.expired")).await?;
        return Ok(MessageFlowOutcome::Handled);
    }

    let Some(destruct_state) = state.destruct_snapshot(&chat_id_str).await else {
        return Ok(MessageFlowOutcome::NotHandled);
    };

    match destruct_state.step {
        DestructStep::AwaitFirstTotp => {
            if let Some(text) = msg.text() {
                let text = text.trim();
                if state.verify_totp(text) {
                    if state
                        .confirm_first_destruct_totp(&chat_id_str, text, Instant::now())
                        .await
                    {
                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback(
                                t!("destruct.confirm_btn"),
                                "a_destroy_confirm",
                            )],
                            vec![InlineKeyboardButton::callback(
                                t!("destruct.cancelled"),
                                "a_destroy_cancel",
                            )],
                        ]);
                        bot.send_message(chat_id, t!("destruct.title_2"))
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard)
                            .await?;
                    }
                } else {
                    bot.send_message(chat_id, t!("destruct.verify_fail"))
                        .await?;
                }
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitSecondTotp => {
            if let Some(text) = msg.text() {
                let text = text.trim();
                if state.verify_totp(text) {
                    match state
                        .confirm_second_destruct_totp(&chat_id_str, text, Instant::now())
                        .await
                    {
                        Err(_) => {
                            bot.send_message(chat_id, t!("destruct.security_warn"))
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                        Ok(true) => {
                            bot.send_message(chat_id, t!("destruct.title_4"))
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                        Ok(false) => {
                            bot.send_message(chat_id, t!("destruct.state_invalid"))
                                .await?;
                        }
                    }
                } else {
                    bot.send_message(chat_id, t!("destruct.verify_fail"))
                        .await?;
                }
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitSecurityFile => {
            let (file_id, file_name) = if let Some(doc) = msg.document() {
                (Some(doc.file.id.clone()), doc.file_name.clone())
            } else if let Some(photos) = msg.photo() {
                (
                    photos.last().map(|p| p.file.id.clone()),
                    Some(t!("destruct.image_label").to_string()),
                )
            } else {
                (None, None)
            };

            if let Some(fid) = file_id {
                let file = bot.get_file(fid.clone()).await?;
                let mut content = Vec::new();
                bot.download_file(&file.path, &mut content)
                    .await
                    .map_err(std::io::Error::other)?;

                let mut hasher = Sha256::new();
                hasher.update(&content);
                let hash_hex = hex::encode(hasher.finalize());

                if let Some(correct) = state.self_destruct_key_hash().await {
                    if hash_hex == correct {
                        let hash_short = if hash_hex.len() > 12 {
                            format!("{}...{}", &hash_hex[..8], &hash_hex[hash_hex.len() - 4..])
                        } else {
                            hash_hex.clone()
                        };
                        let file_display = file_name
                            .map(|n| format!("{} | {}", n, hash_short))
                            .unwrap_or_else(|| hash_short.clone());

                        if state
                            .mark_destruct_file_verified(&chat_id_str, Instant::now())
                            .await
                        {
                            let keyboard = InlineKeyboardMarkup::new(vec![
                                vec![InlineKeyboardButton::callback(
                                    t!("destruct.final_btn"),
                                    "a_destroy_final",
                                )],
                                vec![InlineKeyboardButton::callback(
                                    t!("destruct.cancelled"),
                                    "a_destroy_cancel",
                                )],
                            ]);

                            bot.send_message(
                                chat_id,
                                t!("destruct.file_verify_ok", "0" => file_display),
                            )
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard)
                            .await?;
                        }
                    } else {
                        bot.send_message(chat_id, t!("destruct.file_verify_fail"))
                            .await?;
                    }
                } else {
                    bot.send_message(chat_id, t!("destruct.no_security_file"))
                        .await?;
                }
            } else {
                bot.send_message(chat_id, t!("destruct.file_send_prompt"))
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitConfirm | DestructStep::AwaitFinalConfirm => {
            Ok(MessageFlowOutcome::Handled)
        }
    }
}

pub async fn handle_callback_timeout(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id_str = chat_id.0.to_string();
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.answer_callback_query(q.id.clone())
                .text(t!("destruct.callback_timeout"))
                .await?;
            bot.edit_message_text(chat_id, msg_id, t!("destruct.timeout"))
                .parse_mode(ParseMode::Html)
                .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        TimeoutStatus::Active => Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::NotTracked => Ok(MessageFlowOutcome::NotHandled),
    }
}

pub async fn handle_callback_action(
    bot: &Bot,
    q: &CallbackQuery,
    data: &str,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id_str = chat_id.0.to_string();
    match data {
        "a_destroy_ask" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            state
                .begin_destruct(chat_id_str.clone(), Instant::now())
                .await;
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("destruct.cancelled"),
                "a_destroy_cancel",
            )]]);
            bot.edit_message_text(chat_id, msg_id, t!("destruct.title_1"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_cancel" => {
            if state.cancel_destruct(&chat_id_str).await {
                bot.send_message(chat_id, t!("destruct.cancelled")).await?;
            }
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("destruct.destroy_btn"),
                    "a_destroy_ask",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings"),
                    "m_settings",
                )],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "{}\n\n{}",
                    t!("menu.danger_zone"),
                    t!("menu.danger_zone_desc")
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_confirm" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            if state
                .advance_destruct_step(
                    &chat_id_str,
                    DestructStep::AwaitConfirm,
                    DestructStep::AwaitSecondTotp,
                    Instant::now(),
                )
                .await
            {
                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        t!("destruct.cancelled"),
                        "a_destroy_cancel",
                    )]]);
                bot.edit_message_text(chat_id, msg_id, t!("destruct.title_3"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.state_invalid"))
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_final" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("auth.expired"))
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }

            let snapshot = state.destruct_snapshot(&chat_id_str).await;
            if snapshot.map(|s| s.step) == Some(DestructStep::AwaitFinalConfirm) {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.executing"))
                    .await?;
                bot.edit_message_text(chat_id, msg_id, t!("destruct.final_exec"))
                    .parse_mode(ParseMode::Html)
                    .await?;
                let executor = state.self_destruct_executor();
                aegis::core::security::self_destruct::trigger(executor);
                state.cancel_destruct(&chat_id_str).await;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.state_invalid"))
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        _ => Ok(MessageFlowOutcome::NotHandled),
    }
}
