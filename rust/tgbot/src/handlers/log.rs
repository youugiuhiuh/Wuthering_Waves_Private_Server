use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::logic::log_audit::{LogAudit, SERVICE_SING_BOX, SERVICE_WWPS_CORE};
use crate::utils;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let bot = &ctx.bot;
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;
    let q = &ctx.q;
    let lang = ctx.state.language().await;

    match ctx.data.as_str() {
        "m_log" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("log.xray_log", locale = &lang), "l_xray"),
                    InlineKeyboardButton::callback(t!("log.singbox_log", locale = &lang), "l_box"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("log.back_ops", locale = &lang),
                    "m_ops_center",
                )],
            ]);
            bot.edit_message_text(chat_id, msg_id, t!("log.title", locale = &lang))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(HandlerAction::Done)
        }
        "l_xray" => {
            let status = LogAudit::service_status(SERVICE_WWPS_CORE).await;
            let status_icon = if status.active { "🟢" } else { "🔴" };
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("log.view_recent", locale = &lang),
                    "l_xray_tail",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("log.refresh", locale = &lang),
                    "l_xray",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("log.back_log", locale = &lang),
                    "m_log",
                )],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                t!("log.xray_title", locale = &lang)
                    .replace("%icon%", status_icon)
                    .replace("%status%", &status.status_text)
                    .replace("%service%", SERVICE_WWPS_CORE),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(HandlerAction::Done)
        }
        "l_box" => {
            let status = LogAudit::service_status(SERVICE_SING_BOX).await;
            let status_icon = if status.active { "🟢" } else { "🔴" };
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("log.view_recent", locale = &lang),
                    "l_box_tail",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("log.refresh", locale = &lang),
                    "l_box",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("log.back_log", locale = &lang),
                    "m_log",
                )],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                t!("log.singbox_title", locale = &lang)
                    .replace("%icon%", status_icon)
                    .replace("%status%", &status.status_text)
                    .replace("%service%", SERVICE_SING_BOX),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(HandlerAction::Done)
        }
        "l_xray_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text(t!("log.fetching_xray", locale = &lang))
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_WWPS_CORE, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            t!("log.xray_recent", locale = &lang)
                                .replace("%log%", &utils::escape_html(&log)),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }
        "l_box_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text(t!("log.fetching_singbox", locale = &lang))
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_SING_BOX, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            t!("log.singbox_recent", locale = &lang)
                                .replace("%log%", &utils::escape_html(&log)),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }
        _ => Ok(HandlerAction::Done),
    }
}
