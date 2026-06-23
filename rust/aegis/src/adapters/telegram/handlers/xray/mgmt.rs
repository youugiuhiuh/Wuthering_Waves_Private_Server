use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::xray::ConfigManager;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_mgmt(ctx: &CallbackContext) -> HandlerResult {
    let inbounds = ConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    let mut buttons = Vec::new();

    if inbounds.is_empty() {
        buttons.push(vec![
            InlineKeyboardButton::callback(t!("xray.batch_reality"), "u_batch_init"),
            InlineKeyboardButton::callback(t!("xray.batch_xhttp"), "u_xhttp_batch_init"),
        ]);
        buttons.push(vec![InlineKeyboardButton::callback(
            t!("xray.pq_mgmt"),
            "m_pq_mgmt",
        )]);
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.mgmt_no_cfg"))
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    } else {
        for (i, path) in inbounds.iter().enumerate() {
            let filename = path.split('/').next_back().unwrap_or("Unknown");
            buttons.push(vec![InlineKeyboardButton::callback(
                t!("xray.file_btn", "0" => filename),
                format!("u_l:{}", i),
            )]);
        }
        buttons.push(vec![InlineKeyboardButton::callback(
            t!("xray.del_mgmt_btn"),
            "m_del_cfg",
        )]);
        buttons.push(vec![
            InlineKeyboardButton::callback(t!("xray.batch_reality"), "u_batch_init"),
            InlineKeyboardButton::callback(t!("xray.batch_xhttp"), "u_xhttp_batch_init"),
        ]);
        buttons.push(vec![
            InlineKeyboardButton::callback(t!("xray.batch_kcp"), "u_kcp_init"),
            InlineKeyboardButton::callback(t!("xray.pq_mgmt"), "m_pq_mgmt"),
        ]);
        buttons.push(vec![InlineKeyboardButton::callback(
            t!("menu.back_user"),
            "m_usr",
        )]);
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.mgmt_title"))
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_pq_mgmt(ctx: &CallbackContext) -> HandlerResult {
    let configured = ConfigManager::is_reality_pq_configured();
    let status = if configured {
        t!("xray.pq_status_enabled")
    } else {
        t!("xray.pq_status_disabled")
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            t!("xray.pq_delete"),
            "m_pq_del",
        )],
        vec![InlineKeyboardButton::callback(
            t!("xray.pq_init"),
            "m_pq_init",
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "m_xray_mgmt",
        )],
    ]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.pq_title", "0" => status))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_pq_del(ctx: &CallbackContext) -> HandlerResult {
    match ConfigManager::delete_reality_pq().await {
        Ok(()) => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.pq_del_success"))
                .show_alert(true)
                .await?;
        }
        Err(e) => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.pq_del_fail", "0" => e))
                .show_alert(true)
                .await?;
        }
    }
    Ok(HandlerAction::Redirect("m_pq_mgmt".to_string()))
}

pub(super) async fn handle_pq_init(ctx: &CallbackContext) -> HandlerResult {
    match ConfigManager::generate_reality_pq_keys().await {
        Ok(()) => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.pq_init_success"))
                .show_alert(true)
                .await?;
        }
        Err(e) => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.pq_init_fail", "0" => e))
                .show_alert(true)
                .await?;
        }
    }
    Ok(HandlerAction::Redirect("m_pq_mgmt".to_string()))
}
