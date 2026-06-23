use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::adapters::common::{BotAdapter, TargetId};
use aegis::core::system::SystemMonitor;
use aegis::core::types::IpVersion;
use aegis::core::xray::Proto;
use aegis::core::xray::installer::{RealityInstallOutcome, RealityInstaller};
use rust_i18n::t;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};

mod batch;
mod delete;
mod delete_count;
mod delete_select;
mod mgmt;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "m_xray_mgmt" => mgmt::handle_mgmt(ctx).await,
        "m_pq_mgmt" => mgmt::handle_pq_mgmt(ctx).await,
        "m_pq_del" => mgmt::handle_pq_del(ctx).await,
        "m_pq_init" => mgmt::handle_pq_init(ctx).await,

        "m_del_cfg" => delete::handle_del_cfg(ctx).await,
        d if d.starts_with("cfg_filter:") => delete::handle_cfg_filter(ctx, d).await,
        d if d == "cfg_del_all_confirm" || d.starts_with("cfg_del_all_confirm:") => {
            delete::handle_cfg_del_all_confirm(ctx, d).await
        }
        d if d == "cfg_del_all_exec" || d.starts_with("cfg_del_all_exec:") => {
            delete::handle_cfg_del_all_exec(ctx, d).await
        }

        d if d == "cfg_del_count" || d.starts_with("cfg_del_count:") => {
            delete_count::handle_cfg_del_count(ctx, d).await
        }
        d if d.starts_with("cfg_del_exec_count:") => {
            delete_count::handle_cfg_del_exec_count(ctx, d).await
        }

        d if d == "cfg_del_select" || d.starts_with("cfg_del_select:") => {
            delete_select::handle_cfg_del_select(ctx, d).await
        }
        d if d.starts_with("cfg_del_file:") => delete_select::handle_cfg_del_file(ctx, d).await,
        d if d.starts_with("cfg_del_confirm:") => {
            delete_select::handle_cfg_del_confirm(ctx, d).await
        }

        "a_inst_base" => batch::handle_batch_init(ctx).await,
        "u_batch_init" => batch::handle_batch_init(ctx).await,
        d if d.starts_with("u_batch_ip_init:") => batch::handle_batch_ip_init(ctx, d).await,
        d if d.starts_with("u_batch_exec:") => batch::handle_batch_exec(ctx, d).await,
        "u_xhttp_batch_init" => batch::handle_xhttp_batch_init(ctx).await,
        d if d.starts_with("u_xhttp_batch_ip_init:") => {
            batch::handle_xhttp_batch_ip_init(ctx, d).await
        }
        d if d.starts_with("u_xhttp_batch_exec:") => batch::handle_xhttp_batch_exec(ctx, d).await,
        "u_kcp_init" => batch::handle_kcp_init(ctx).await,
        d if d.starts_with("u_kcp_cat:") => batch::handle_kcp_cat(ctx, d).await,
        d if d.starts_with("u_kcp_add:") => batch::handle_kcp_add(ctx, d).await,
        d if d.starts_with("u_kcp_more:") => batch::handle_kcp_more(ctx, d).await,
        d if d.starts_with("u_kcp_mcat:") => batch::handle_kcp_mcat(ctx, d).await,
        d if d.starts_with("u_kcp_push:") => batch::handle_kcp_push(ctx, d).await,
        d if d.starts_with("u_kcp_done:") => batch::handle_kcp_done(ctx, d).await,
        d if d.starts_with("u_kcp_ip:") => batch::handle_kcp_ip(ctx, d).await,
        d if d.starts_with("u_kcp_ok:") => batch::handle_kcp_ok(ctx, d).await,
        d if d.starts_with("u_l:") => batch::handle_user_list(ctx, d).await,
        d if d.starts_with("u_d:") => batch::handle_user_del(ctx, d).await,
        d if d.starts_with("u_d_confirm:") => batch::handle_user_del_confirm(ctx, d).await,

        _ => Ok(HandlerAction::Done),
    }
}

async fn show_reality_batch_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    proto: Proto,
) -> ResponseResult<()> {
    let (ip_prefix, title) = match proto {
        Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
        Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        "🌐 IPv4 (0.0.0.0)",
        format!("{}4", ip_prefix),
    )]];

    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("{}6", ip_prefix),
        ));

        if proto == Proto::XHTTP {
            buttons.push(vec![
                InlineKeyboardButton::callback(t!("xray.split_v6_up"), format!("{}s6", ip_prefix)),
                InlineKeyboardButton::callback(t!("xray.split_v4_up"), format!("{}s4", ip_prefix)),
            ]);
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("menu.back_user"),
        "m_usr",
    )]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        t!(
            "xray.batch_title",
            "0" => title,
            "1" => t!("xray.batch_security"),
            "2" => t!("xray.batch_step_ip")
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

async fn show_reality_qty_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    ip_version: IpVersion,
    proto: Proto,
) -> ResponseResult<()> {
    let ip_ver_code = match ip_version {
        IpVersion::IPv4 => "4",
        IpVersion::IPv6 => "6",
        IpVersion::SplitStackV6Primary => "s6",
        IpVersion::SplitStackV4Primary => "s4",
    };
    let ip_display = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        IpVersion::SplitStackV6Primary => &t!("xray.split_v6_up"),
        IpVersion::SplitStackV4Primary => &t!("xray.split_v4_up"),
    };

    let (exec_prefix, title) = match proto {
        Proto::Vision => ("u_batch_exec:", "Reality"),
        Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let buttons = vec![
        vec![
            InlineKeyboardButton::callback("1", format!("{exec_prefix}{ip_ver_code}:1")),
            InlineKeyboardButton::callback("3", format!("{exec_prefix}{ip_ver_code}:3")),
            InlineKeyboardButton::callback("5", format!("{exec_prefix}{ip_ver_code}:5")),
        ],
        vec![
            InlineKeyboardButton::callback("10", format!("{exec_prefix}{ip_ver_code}:10")),
            InlineKeyboardButton::callback("20", format!("{exec_prefix}{ip_ver_code}:20")),
            InlineKeyboardButton::callback("50", format!("{exec_prefix}{ip_ver_code}:50")),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back_user"),
            "m_usr",
        )],
    ];

    bot.edit_message_text(
        chat_id,
        msg_id,
        t!(
            "xray.batch_title",
            "0" => title,
            "1" => "",
            "2" => t!("xray.batch_step_qty", "0" => ip_display)
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

fn trigger_reality_auto_init(
    adapter: Arc<dyn BotAdapter>,
    bot: Bot,
    chat_id: ChatId,
    msg_id: MessageId,
) {
    let target = TargetId(chat_id.0.to_string());
    tokio::spawn(async move {
        let aegis_msg_id = aegis::adapters::common::MessageId(msg_id.0.to_string());
        match RealityInstaller::run(adapter.as_ref(), &target, Some(&aegis_msg_id)).await {
            Ok(RealityInstallOutcome::AlreadyReady) => {
                let _ = show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision).await;
            }
            Ok(RealityInstallOutcome::Completed) => {
                let _ = show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision).await;
                let _ = bot
                    .send_message(chat_id, t!("xray.reality_ready"))
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Ok(RealityInstallOutcome::InProgress) => {}
            Err(e) => {
                let _ = bot
                    .send_message(chat_id, t!("xray.reality_init_fail", "0" => e))
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
    });
}
