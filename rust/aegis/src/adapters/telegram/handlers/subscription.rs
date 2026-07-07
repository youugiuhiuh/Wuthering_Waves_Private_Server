use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::state::{AppState, SubSetupState, SubSetupStep};
use aegis::core::paths;
use aegis::core::subscription::cert::TlsMode;
use aegis::core::subscription::deploy::{self, DeployParams};
use rust_i18n::t;
use std::path::Path;
use std::time::Duration;
use teloxide::payloads::SendMessageSetters;
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
        d if d.starts_with("sub_sel:") => handle_setup_select(ctx, d).await,
        d if d.starts_with("sub_tls:") => handle_tls_select(ctx, d).await,
        "sub_confirm" => handle_deploy_execute(ctx).await,
        "sub_cancel" => handle_setup_cancel(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

pub async fn handle_text_input(
    bot: &Bot,
    chat_id: ChatId,
    state: &AppState,
    text: &str,
) -> Result<bool, teloxide::RequestError> {
    let chat_id_str = chat_id.0.to_string();
    let Some(mut setup) = state.sub_setup_status(&chat_id_str).await else {
        return Ok(false);
    };

    match setup.step {
        SubSetupStep::EnterDomain => {
            if text.is_empty() || text.contains(' ') || !text.contains('.') {
                bot.send_message(chat_id, t!("sub.setup_q_domain_input"))
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(true);
            }
            setup.domain = text.trim().to_string();
            setup.step = SubSetupStep::EnterPort;
            state.insert_sub_setup(chat_id_str, setup).await;
            bot.send_message(chat_id, t!("sub.setup_q_port"))
                .parse_mode(ParseMode::Html)
                .await?;
        }
        SubSetupStep::EnterPort => {
            let port: u16 = match text.trim().parse() {
                Ok(p) if (1024..=65535).contains(&p) => p,
                _ => 8443,
            };
            setup.port = port;
            setup.step = SubSetupStep::EnterRateLimit;
            state.insert_sub_setup(chat_id_str, setup).await;
            bot.send_message(chat_id, t!("sub.setup_q_rate"))
                .parse_mode(ParseMode::Html)
                .await?;
        }
        SubSetupStep::EnterRateLimit => {
            let rate: u32 = match text.trim().parse() {
                Ok(r) if (1..=100).contains(&r) => r,
                _ => 10,
            };
            setup.rate_limit = rate;
            setup.step = SubSetupStep::ChooseTls;
            state.insert_sub_setup(chat_id_str, setup).await;
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_le"),
                    "sub_tls:le",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_ip"),
                    "sub_tls:ip",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_self"),
                    "sub_tls:self",
                )],
                vec![InlineKeyboardButton::callback(t!("menu.back"), "m_sub")],
            ]);
            bot.send_message(chat_id, t!("sub.setup_q_cert"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        SubSetupStep::ChooseDomain | SubSetupStep::ChooseTls | SubSetupStep::Confirm => {}
    }
    Ok(true)
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

async fn handle_setup_select(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let choice = data.strip_prefix("sub_sel:").unwrap_or("");
    let chat_id = ctx.chat_id.0.to_string();
    let has_domain = choice == "domain";
    let step = if has_domain {
        SubSetupStep::EnterDomain
    } else {
        SubSetupStep::EnterPort
    };
    let setup = SubSetupState {
        step,
        has_domain,
        domain: String::new(),
        port: 8443,
        rate_limit: 10,
        tls_mode: 0,
    };
    ctx.state.insert_sub_setup(chat_id, setup).await;
    let msg = if has_domain {
        t!("sub.setup_q_domain_input")
    } else {
        t!("sub.setup_q_port")
    };
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, msg)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_tls_select(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let tls_choice = data.strip_prefix("sub_tls:").unwrap_or("");
    let chat_id = ctx.chat_id.0.to_string();
    let Some(mut setup) = ctx.state.sub_setup_status(&chat_id).await else {
        return Ok(HandlerAction::Done);
    };
    setup.tls_mode = match tls_choice {
        "le" => 0,
        "ip" => 1,
        "self" => 2,
        _ => 0,
    };
    setup.step = SubSetupStep::Confirm;
    ctx.state.insert_sub_setup(chat_id.clone(), setup).await;

    let Some(setup) = ctx.state.sub_setup_status(&chat_id).await else {
        return Ok(HandlerAction::Done);
    };
    let domain_display = if setup.has_domain {
        setup.domain.as_str()
    } else {
        "IP only"
    };
    let tls_name = match setup.tls_mode {
        0 => t!("sub.setup_tls_name_le"),
        1 => t!("sub.setup_tls_name_ip"),
        _ => t!("sub.setup_tls_name_self"),
    };
    let self_signed_warn = if setup.tls_mode == 2 {
        format!("\n\n{}", t!("sub.risk_self_signed"))
    } else {
        String::new()
    };
    let summary = t!(
        "sub.setup_confirm_title",
        "0" => domain_display,
        "1" => setup.port.to_string(),
        "2" => setup.rate_limit.to_string(),
        "3" => tls_name,
    );
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            t!("sub.setup_confirm_btn"),
            "sub_confirm",
        )],
        vec![InlineKeyboardButton::callback(
            t!("sub.setup_cancel"),
            "sub_cancel",
        )],
    ]);
    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            format!("{}{}", summary, self_signed_warn),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_deploy_execute(ctx: &CallbackContext) -> HandlerResult {
    let chat_id = ctx.chat_id.0.to_string();
    let Some(setup) = ctx.state.sub_setup_status(&chat_id).await else {
        return Ok(HandlerAction::Done);
    };
    ctx.state.remove_sub_setup(&chat_id).await;

    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let tm = tm.clone();

    let tls_mode = match setup.tls_mode {
        0 => TlsMode::DomainAcme,
        1 => TlsMode::IpAcme,
        _ => TlsMode::SelfSigned,
    };
    let params = DeployParams {
        domain: if setup.domain.is_empty() {
            "0.0.0.0".to_string()
        } else {
            setup.domain.clone()
        },
        port: setup.port,
        rate_limit: setup.rate_limit,
        tls_mode,
    };

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("sub.setup_step_download"))
        .parse_mode(ParseMode::Html)
        .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    match deploy::run_deploy(&params, &tm).await {
        Ok(result) => {
            let success_msg = t!(
                "sub.setup_success",
                "0" => &params.domain,
                "1" => params.port.to_string(),
                "2" => &result.token,
            );
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                "m_sub",
            )]]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, success_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        Err(e) => {
            let fail_msg = t!("sub.setup_fail", "0" => e);
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                "m_sub",
            )]]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, fail_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_setup_cancel(ctx: &CallbackContext) -> HandlerResult {
    let chat_id = ctx.chat_id.0.to_string();
    ctx.state.remove_sub_setup(&chat_id).await;
    handle_main_menu(ctx).await
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
                    let mask: String = t.token.chars().take(4).collect();
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
            let mask: String = info.token.chars().take(4).collect();
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
