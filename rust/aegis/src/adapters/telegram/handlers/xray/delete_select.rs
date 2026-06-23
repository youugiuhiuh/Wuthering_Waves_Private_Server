use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::utils;
use aegis::core::xray::{ConfigManager, Proto};
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_cfg_del_select(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let filter = data.strip_prefix("cfg_del_select:").unwrap_or("all");
    let files = if filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };
    let filter_label = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let mut buttons = Vec::new();
    for (i, path) in files.iter().enumerate().take(50) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        buttons.push(vec![InlineKeyboardButton::callback(
            format!("🗑 {}", filename),
            format!("cfg_del_file:{}:{}", filter, i),
        )]);
    }
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "cfg_filter:all",
    )]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.del_select_title", "0" => filter_label),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_del_file(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };

    if let Some(path) = files.get(idx) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        let keyboard = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "⚠️ Confirm Delete",
                format!("cfg_del_confirm:{}:{}", filter, idx),
            )],
            vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                format!("cfg_del_select:{}", filter),
            )],
        ]);
        ctx.bot
            .edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                t!("xray.del_confirm_msg", "0" => utils::escape_html(filename)),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.del_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_del_confirm(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };

    if let Err(e) = utils::validate_idx(idx, files.len(), &t!("xray.del_label")) {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(format!("❌ {}", e))
            .await?;
        return Ok(HandlerAction::Done);
    }

    if let Some(path) = files.get(idx) {
        let _ = ConfigManager::delete_specific_configuration(path).await;
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.del_success"))
            .show_alert(true)
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.del_nonexist"))
            .show_alert(true)
            .await?;
    }

    Ok(HandlerAction::Redirect(format!(
        "cfg_del_select:{}",
        filter
    )))
}
