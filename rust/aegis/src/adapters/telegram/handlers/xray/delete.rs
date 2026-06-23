use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::xray::{ConfigManager, Proto};
use rust_i18n::t;
use std::fs;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_del_cfg(ctx: &CallbackContext) -> HandlerResult {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("xray.filter_all"), "cfg_filter:all"),
            InlineKeyboardButton::callback(t!("xray.filter_reality"), "cfg_filter:reality"),
            InlineKeyboardButton::callback(t!("xray.filter_xhttp"), "cfg_filter:xhttp"),
            InlineKeyboardButton::callback(t!("xray.filter_kcp"), "cfg_filter:kcp"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_all"),
            "cfg_del_all_confirm:all",
        )],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_count"),
            "cfg_del_count:all",
        )],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_select"),
            "cfg_del_select:all",
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "m_xray_mgmt",
        )],
    ]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.del_title", "0" => t!("xray.filter_all")),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_filter(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let filter = data.strip_prefix("cfg_filter:").unwrap_or("all");
    let filter_label_val = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("xray.filter_all"), "cfg_filter:all"),
            InlineKeyboardButton::callback(t!("xray.filter_reality"), "cfg_filter:reality"),
            InlineKeyboardButton::callback(t!("xray.filter_xhttp"), "cfg_filter:xhttp"),
            InlineKeyboardButton::callback(t!("xray.filter_kcp"), "cfg_filter:kcp"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_all"),
            format!("cfg_del_all_confirm:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_count"),
            format!("cfg_del_count:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(
            t!("xray.del_select"),
            format!("cfg_del_select:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "m_xray_mgmt",
        )],
    ]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.del_title", "0" => filter_label_val),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_del_all_confirm(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let filter = data.strip_prefix("cfg_del_all_confirm:").unwrap_or("all");
    let filter_type_label = match filter {
        "reality" => t!("xray.type_reality"),
        "xhttp" => t!("xray.type_xhttp"),
        "kcp" => t!("xray.type_kcp"),
        _ => t!("xray.type_all"),
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            t!("xray.confirm_clear_btn"),
            format!("cfg_del_all_exec:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(t!("menu.back"), "m_del_cfg")],
    ]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.confirm_del_all", "0" => filter_type_label),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_del_all_exec(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let filter = data.strip_prefix("cfg_del_all_exec:").unwrap_or("all");
    let count = if filter == "all" {
        ConfigManager::delete_all_configurations()
            .await
            .unwrap_or(0)
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("xray.del_unknown_filter"))
                    .await?;
                return Ok(HandlerAction::Redirect("m_del_cfg".to_string()));
            }
        };
        let files = ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default();
        let count = files.len();
        for f in &files {
            let _ = fs::remove_file(f);
        }
        if count > 0 {
            let _ = MaintenanceManager::reload_core().await;
        }
        count
    };
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("xray.del_success_all", "0" => count))
        .show_alert(true)
        .await?;

    Ok(HandlerAction::Redirect("m_del_cfg".to_string()))
}
