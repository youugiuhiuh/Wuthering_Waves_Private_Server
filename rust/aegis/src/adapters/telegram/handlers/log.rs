use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::utils;
use aegis::core::system::log_audit::{LogAudit, SERVICE_SING_BOX, SERVICE_WWPS_CORE};
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let bot = &ctx.bot;
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;
    let q = &ctx.q;

    match ctx.data.as_str() {
        "m_log" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("log.xray_btn"), "l_xray"),
                    InlineKeyboardButton::callback(t!("log.box_btn"), "l_box"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops"),
                    "m_ops_center",
                )],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!("{}\n{}", t!("menu.log_audit"), t!("menu.log_audit_desc")),
            )
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
                    t!("log.view_tail"),
                    "l_xray_tail",
                )],
                vec![InlineKeyboardButton::callback(t!("menu.refresh"), "l_xray")],
                vec![InlineKeyboardButton::callback(t!("log.back_log"), "m_log")],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                t!("log.xray_log_title", "0" => status_icon, "1" => status.status_text, "2" => SERVICE_WWPS_CORE),
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
                    t!("log.view_tail"),
                    "l_box_tail",
                )],
                vec![InlineKeyboardButton::callback(t!("menu.refresh"), "l_box")],
                vec![InlineKeyboardButton::callback(t!("log.back_log"), "m_log")],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                t!("log.box_log_title", "0" => status_icon, "1" => status.status_text, "2" => SERVICE_SING_BOX),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(HandlerAction::Done)
        }
        "l_xray_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text(t!("log.fetching_xray"))
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_WWPS_CORE, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            t!("log.xray_tail_title", "0" => utils::escape_html(&log)),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }
        "l_box_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text(t!("log.fetching_box"))
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_SING_BOX, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            t!("log.box_tail_title", "0" => utils::escape_html(&log)),
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
