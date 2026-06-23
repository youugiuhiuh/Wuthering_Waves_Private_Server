use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::xray::{ConfigManager, Proto};
use rust_i18n::t;
use std::fs;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_cfg_del_count(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let filter = data.strip_prefix("cfg_del_count:").unwrap_or("all");
    let filter_label = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("10", format!("cfg_del_exec_count:{}:10", filter)),
            InlineKeyboardButton::callback("50", format!("cfg_del_exec_count:{}:50", filter)),
        ],
        vec![
            InlineKeyboardButton::callback("100", format!("cfg_del_exec_count:{}:100", filter)),
            InlineKeyboardButton::callback("500", format!("cfg_del_exec_count:{}:500", filter)),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "cfg_filter:all",
        )],
    ]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.del_count_title", "0" => filter_label),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_cfg_del_exec_count(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let n: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

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

    let mut file_with_time = Vec::new();
    for f in files {
        if let Ok(meta) = std::fs::metadata(&f)
            && let Ok(time) = meta.modified()
        {
            file_with_time.push((f, time));
        }
    }
    file_with_time.sort_by_key(|a| a.1);

    let to_delete = file_with_time.iter().take(n);
    let mut deleted_count = 0;
    for (f, _) in to_delete {
        if fs::remove_file(f).is_ok() {
            deleted_count += 1;
        }
    }
    if deleted_count > 0 {
        let _ = MaintenanceManager::reload_core().await;
    }
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("xray.del_success_count", "0" => deleted_count))
        .show_alert(true)
        .await?;

    Ok(HandlerAction::Redirect(format!("cfg_del_count:{}", filter)))
}
