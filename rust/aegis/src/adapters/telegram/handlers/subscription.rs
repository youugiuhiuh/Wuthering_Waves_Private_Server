use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::paths;
use rust_i18n::t;
use std::path::Path;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    match ctx.data.as_str() {
        "m_sub" => handle_main_menu(ctx).await,
        "sub_status" => handle_status(ctx).await,
        "sub_setup" => handle_setup(ctx).await,
        "sub_tcreate" => handle_token_create(ctx).await,
        "sub_tlist" => handle_token_list(ctx).await,
        d if d.starts_with("sub_tinfo:") => handle_token_info(ctx, d).await,
        d if d.starts_with("sub_trevoke:") => handle_token_revoke(ctx, d).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_main_menu(ctx: &CallbackContext) -> HandlerResult {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("menu.sub_status"), "sub_status"),
            InlineKeyboardButton::callback(t!("menu.sub_setup"), "sub_setup"),
        ],
        vec![
            InlineKeyboardButton::callback(t!("menu.sub_create"), "sub_tcreate"),
            InlineKeyboardButton::callback(t!("menu.sub_list"), "sub_tlist"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back_main"),
            "m_main",
        )],
    ]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_service"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_status(ctx: &CallbackContext) -> HandlerResult {
    let installed = Path::new(paths::sub_server::BIN).exists();
    let text = if installed {
        t!("menu.sub_installed", "0" => paths::sub_server::SERVICE, "1" => "8443", "2" => "N/A")
            .into_owned()
    } else {
        t!("menu.sub_not_installed").into_owned()
    };
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "m_sub",
    )]]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_setup(ctx: &CallbackContext) -> HandlerResult {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("sub.setup_q_domain_yes"), "sub_sel:domain"),
            InlineKeyboardButton::callback(t!("sub.setup_q_domain_no"), "sub_sel:ip"),
        ],
        vec![InlineKeyboardButton::callback(t!("menu.back"), "m_sub")],
    ]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("sub.setup_welcome"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_token_create(ctx: &CallbackContext) -> HandlerResult {
    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let text = match tm.create_token("manual", &[]) {
        Ok(token) => t!("sub.token_created", "0" => &token.token).into_owned(),
        Err(e) => t!("sub.setup_fail", "0" => e.to_string()).into_owned(),
    };
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_token_list(ctx: &CallbackContext) -> HandlerResult {
    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "m_sub",
    )]]);
    let text = match tm.list_tokens(1, 20) {
        Ok((tokens, _total)) => {
            if tokens.is_empty() {
                t!("sub.token_list_empty").into_owned()
            } else {
                let mut lines = Vec::new();
                for t in &tokens {
                    let mask: String = t.token.chars().take(8).collect();
                    let status = if t.revoked {
                        t!("sub.token_status_revoked")
                    } else {
                        t!("sub.token_status_active")
                    };
                    lines.push(format!("• <code>{}****</code> {}", mask, status));
                }
                t!("sub.token_list_title", "0" => lines.join("\n")).into_owned()
            }
        }
        Err(e) => t!("sub.setup_fail", "0" => e.to_string()).into_owned(),
    };
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_token_info(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let token = data.strip_prefix("sub_tinfo:").unwrap_or("");
    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "m_sub",
    )]]);
    let text = match tm.get_token_info(token) {
        Ok((info, count)) => {
            let mask: String = info.token.chars().take(8).collect();
            let status = if info.revoked {
                t!("sub.token_status_revoked")
            } else {
                t!("sub.token_status_active")
            };
            let created = info.created_at.to_string();
            let expires = if info.expires_at > 0 {
                info.expires_at.to_string()
            } else {
                "None".to_string()
            };
            t!(
                "sub.token_info_title",
                "0" => mask,
                "1" => status,
                "2" => created,
                "3" => expires,
                "4" => count.to_string(),
                "5" => format!("https://.../sub/{}", mask)
            )
            .into_owned()
        }
        Err(_) => t!("sub.token_not_found").into_owned(),
    };
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_token_revoke(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let token = data.strip_prefix("sub_trevoke:").unwrap_or("");
    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let text = match tm.revoke_token(token) {
        Ok(()) => {
            let mask: String = token.chars().take(8).collect();
            t!("sub.token_revoked", "0" => mask).into_owned()
        }
        Err(e) => t!("sub.token_revoke_fail", "0" => e.to_string()).into_owned(),
    };
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(HandlerAction::Done)
}
