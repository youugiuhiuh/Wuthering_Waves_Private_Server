use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::utils;
use aegis::core::system::log_audit::{LogAudit, SERVICE_SING_BOX, SERVICE_WWPS_CORE};
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
                    InlineKeyboardButton::callback("🅧 Xray-core 日志", "l_xray"),
                    InlineKeyboardButton::callback("📦 Sing-box 日志", "l_box"),
                ],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回运维中心",
                    "m_ops_center",
                )],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                "📄 日志审计\n通过 systemd journal 获取服务日志:",
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
                    "📝 查看最近日志",
                    "l_xray_tail",
                )],
                vec![InlineKeyboardButton::callback("🔄 刷新", "l_xray")],
                vec![InlineKeyboardButton::callback("⬅️ 返回日志审计", "m_log")],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🅧 Xray-core 日志\n\n状态: {} {} | 日志来源: journalctl -u {}",
                    status_icon, status.status_text, SERVICE_WWPS_CORE
                ),
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
                    "📝 查看最近日志",
                    "l_box_tail",
                )],
                vec![InlineKeyboardButton::callback("🔄 刷新", "l_box")],
                vec![InlineKeyboardButton::callback("⬅️ 返回日志审计", "m_log")],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📦 Sing-box 日志\n\n状态: {} {} | 日志来源: journalctl -u {}",
                    status_icon, status.status_text, SERVICE_SING_BOX
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(HandlerAction::Done)
        }
        "l_xray_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text("📝 正在获取 Xray-core 日志...")
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_WWPS_CORE, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            format!(
                                "🅧 Xray-core 最近日志:\n\n<pre>{}</pre>",
                                utils::escape_html(&log)
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            });
            Ok(HandlerAction::Done)
        }
        "l_box_tail" => {
            bot.answer_callback_query(q.id.clone())
                .text("📝 正在获取 Sing-box 日志...")
                .await?;
            let bot_c = bot.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_SING_BOX, 50).await {
                    let _ = bot_c
                        .send_message(
                            chat_id,
                            format!(
                                "📦 Sing-box 最近日志:\n\n<pre>{}</pre>",
                                utils::escape_html(&log)
                            ),
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
