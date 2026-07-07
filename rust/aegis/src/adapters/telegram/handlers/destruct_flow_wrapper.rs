use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::payloads::EditMessageTextSetters;

use aegis::app::destruct_flow::MessageFlowOutcome;
use aegis::app::state::{AppState, DestructStep, TimeoutStatus};

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
            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback(
                    t!("destruct.cancelled"),
                    "a_destroy_cancel",
                ),
            ]]);
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
            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![
                vec![teloxide::types::InlineKeyboardButton::callback(
                    t!("destruct.destroy_btn"),
                    "a_destroy_ask",
                )],
                vec![teloxide::types::InlineKeyboardButton::callback(
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
                let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                    teloxide::types::InlineKeyboardButton::callback(
                        t!("destruct.cancelled"),
                        "a_destroy_cancel",
                    ),
                ]]);
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
