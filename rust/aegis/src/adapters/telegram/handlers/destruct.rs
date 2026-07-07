use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::destruct_flow::{
    self, ButtonSpec, DestructInput, DestructOutput, MessageFlowOutcome,
};
use crate::app::state::{AppState, TimeoutStatus};
use rust_i18n::t;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle_message_flow(
    bot: &Bot,
    msg: &Message,
    user_id: i64,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id = msg.chat.id;
    let chat_id_str = chat_id.0.to_string();

    // Timeout check
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

    // Extract content from message
    let input = if let Some(text) = msg.text() {
        DestructInput::Text(text.to_string())
    } else if let Some(doc) = msg.document() {
        let file = bot.get_file(doc.file.id.clone()).await?;
        let mut content = Vec::new();
        bot.download_file(&file.path, &mut content)
            .await
            .map_err(|e| std::io::Error::other(e))?;
        DestructInput::File(content)
    } else if let Some(photos) = msg.photo() {
        if let Some(p) = photos.last() {
            let file = bot.get_file(p.file.id.clone()).await?;
            let mut content = Vec::new();
            bot.download_file(&file.path, &mut content)
                .await
                .map_err(|e| std::io::Error::other(e))?;
            DestructInput::File(content)
        } else {
            return Ok(MessageFlowOutcome::NotHandled);
        }
    } else {
        return Ok(MessageFlowOutcome::NotHandled);
    };

    let (outcome, outputs) =
        destruct_flow::handle_input(state, &chat_id_str, user_id, input, Instant::now()).await;

    for output in outputs {
        render_message_output(bot, chat_id, &output).await?;
    }

    Ok(outcome)
}

async fn render_message_output(
    bot: &Bot,
    chat_id: ChatId,
    output: &DestructOutput,
) -> ResponseResult<()> {
    match output {
        DestructOutput::Prompt { text, buttons } => {
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(convert_buttons(buttons))
                .await?;
        }
        DestructOutput::Text(text) => {
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        DestructOutput::Execute | DestructOutput::Noop => {}
    }
    Ok(())
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
    use crate::app::destruct_flow::BTN_DESTROY_CANCEL;
    use crate::app::destruct_flow::BTN_DESTROY_ASK;
    let chat_id_str = chat_id.0.to_string();

    // Special case: cancel restores Telegram-specific menu
    if data == BTN_DESTROY_CANCEL {
        let cancelled = state.cancel_destruct(&chat_id_str).await;
        if cancelled {
            bot.send_message(chat_id, t!("destruct.cancelled")).await?;
        }
        let keyboard = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                t!("destruct.destroy_btn"),
                BTN_DESTROY_ASK.to_string(),
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
        return Ok(MessageFlowOutcome::Handled);
    }

    // Generic: delegate to handle_input
    let user_id = q.from.id.0 as i64;
    let (outcome, outputs) = destruct_flow::handle_input(
        state,
        &chat_id_str,
        user_id,
        DestructInput::Button(data.to_string()),
        Instant::now(),
    ).await;

    for output in outputs {
        match &output {
            DestructOutput::Execute => {
                bot.answer_callback_query(q.id.clone())
                    .text(t!("destruct.executing"))
                    .await?;
                bot.edit_message_text(chat_id, msg_id, t!("destruct.final_exec"))
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            DestructOutput::Prompt { text, buttons } => {
                bot.edit_message_text(chat_id, msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(convert_buttons(buttons))
                    .await?;
            }
            DestructOutput::Text(text) => {
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            DestructOutput::Noop => {}
        }
    }

    Ok(outcome)
}

fn convert_buttons(buttons: &[Vec<ButtonSpec>]) -> InlineKeyboardMarkup {
    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .iter()
        .map(|row| {
            row.iter()
                .map(|btn| InlineKeyboardButton::callback(&btn.text, &btn.action))
                .collect()
        })
        .collect();
    InlineKeyboardMarkup::new(rows)
}
