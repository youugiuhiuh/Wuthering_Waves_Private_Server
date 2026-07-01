use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};
use crate::utils;
use aegis::adapters::common::{MessageContent, TargetId};
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::types::IpVersion;
use aegis::core::xray::{ConfigManager, KcpMask, Proto};
use rust_i18n::t;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_batch_init(ctx: &CallbackContext) -> HandlerResult {
    if MaintenanceManager::is_reality_base_ready().await {
        show_reality_batch_prompt(&ctx.bot, ctx.chat_id, ctx.msg_id, Proto::Vision).await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.preparing_reality"))
            .await?;
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.init_reality"))
            .parse_mode(ParseMode::Html)
            .await?;
        trigger_reality_auto_init(
            ctx.state.adapter.clone(),
            ctx.bot.clone(),
            ctx.chat_id,
            ctx.msg_id,
        );
    }
    Ok(HandlerAction::Done)
}

pub(super) async fn handle_xhttp_batch_init(ctx: &CallbackContext) -> HandlerResult {
    if MaintenanceManager::is_reality_base_ready().await {
        show_reality_batch_prompt(&ctx.bot, ctx.chat_id, ctx.msg_id, Proto::XHTTP).await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.preparing_reality"))
            .await?;
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.init_reality"))
            .parse_mode(ParseMode::Html)
            .await?;
        trigger_reality_auto_init(
            ctx.state.adapter.clone(),
            ctx.bot.clone(),
            ctx.chat_id,
            ctx.msg_id,
        );
    }
    Ok(HandlerAction::Done)
}

pub(super) async fn handle_batch_ip_init(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let prefix = "u_batch_ip_init:";
    let proto = Proto::Vision;
    let ip_ver_code = data.strip_prefix(prefix).unwrap_or("");
    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };
    show_reality_qty_prompt(&ctx.bot, ctx.chat_id, ctx.msg_id, ip_version, proto).await?;
    Ok(HandlerAction::Done)
}

pub(super) async fn handle_xhttp_batch_ip_init(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let prefix = "u_xhttp_batch_ip_init:";
    let proto = Proto::XHTTP;
    let ip_ver_code = data.strip_prefix(prefix).unwrap_or("");
    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };
    show_reality_qty_prompt(&ctx.bot, ctx.chat_id, ctx.msg_id, ip_version, proto).await?;
    Ok(HandlerAction::Done)
}

pub(super) async fn handle_batch_exec(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let prefix = "u_batch_exec:";
    let proto = Proto::Vision;
    let parts: Vec<&str> = data
        .strip_prefix(prefix)
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let ip_ver_code = parts[0];
    let n: usize = parts[1].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };

    if !MaintenanceManager::is_reality_base_ready().await {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.base_missing"))
            .await?;
        trigger_reality_auto_init(
            ctx.state.adapter.clone(),
            ctx.bot.clone(),
            ctx.chat_id,
            ctx.msg_id,
        );
        return Ok(HandlerAction::Done);
    }

    let ip_str: std::borrow::Cow<'_, str> = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up"),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up"),
    };

    let proto_str = match proto {
        Proto::Vision => "Reality",
        Proto::XHTTP => "XHTTP",
        Proto::Kcp => "KCP",
    };

    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("xray.gen_progress", "0" => n, "1" => proto_str, "2" => ip_str))
        .await?;

    let res = match proto {
        Proto::Vision => ConfigManager::batch_create_reality_vision_enhanced(n, ip_version).await,
        Proto::XHTTP => ConfigManager::batch_create_xhttp_reality_enhanced(n, ip_version).await,
        Proto::Kcp => {
            unreachable!("KCP uses separate batch handler")
        }
    };

    let adapter = ctx.state.adapter.clone();
    let target = TargetId(ctx.chat_id.0.to_string());

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.batch_done",
                "0" => result.created_count,
                "1" => ip_str
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_config_file", "0" => filename)
                ));
            }

            if let Some(backup_file) = result.backup_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_backup_file", "0" => backup_file)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: result_msg,
                        markup: None,
                    },
                )
                .await
            {
                message_ids.push(msg.0);
            }

            let adapter_clone = adapter.clone();
            let target_clone = target.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                for id_str in message_ids {
                    let mid = aegis::adapters::common::MessageId(id_str);
                    if let Err(e) = adapter_clone.delete_message(&target_clone, &mid).await {
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("未找到 Reality 配置文件") {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.master_missing").to_string(),
                            markup: None,
                        },
                    )
                    .await;
                trigger_reality_auto_init(
                    ctx.state.adapter.clone(),
                    ctx.bot.clone(),
                    ctx.chat_id,
                    ctx.msg_id,
                );
            } else {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.gen_fail", "0" => err_msg).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_xhttp_batch_exec(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let prefix = "u_xhttp_batch_exec:";
    let proto = Proto::XHTTP;
    let parts: Vec<&str> = data
        .strip_prefix(prefix)
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let ip_ver_code = parts[0];
    let n: usize = parts[1].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };

    if !MaintenanceManager::is_reality_base_ready().await {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.base_missing"))
            .await?;
        trigger_reality_auto_init(
            ctx.state.adapter.clone(),
            ctx.bot.clone(),
            ctx.chat_id,
            ctx.msg_id,
        );
        return Ok(HandlerAction::Done);
    }

    let ip_str: std::borrow::Cow<'_, str> = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up"),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up"),
    };

    let proto_str = match proto {
        Proto::Vision => "Reality",
        Proto::XHTTP => "XHTTP",
        Proto::Kcp => "KCP",
    };

    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("xray.gen_progress", "0" => n, "1" => proto_str, "2" => ip_str))
        .await?;

    let res = match proto {
        Proto::Vision => ConfigManager::batch_create_reality_vision_enhanced(n, ip_version).await,
        Proto::XHTTP => ConfigManager::batch_create_xhttp_reality_enhanced(n, ip_version).await,
        Proto::Kcp => {
            unreachable!("KCP uses separate batch handler")
        }
    };

    let adapter = ctx.state.adapter.clone();
    let target = TargetId(ctx.chat_id.0.to_string());

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.batch_done",
                "0" => result.created_count,
                "1" => ip_str
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_config_file", "0" => filename)
                ));
            }

            if let Some(backup_file) = result.backup_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_backup_file", "0" => backup_file)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: result_msg,
                        markup: None,
                    },
                )
                .await
            {
                message_ids.push(msg.0);
            }

            let adapter_clone = adapter.clone();
            let target_clone = target.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                for id_str in message_ids {
                    let mid = aegis::adapters::common::MessageId(id_str);
                    if let Err(e) = adapter_clone.delete_message(&target_clone, &mid).await {
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("未找到 Reality 配置文件") {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.master_missing").to_string(),
                            markup: None,
                        },
                    )
                    .await;
                trigger_reality_auto_init(
                    ctx.state.adapter.clone(),
                    ctx.bot.clone(),
                    ctx.chat_id,
                    ctx.msg_id,
                );
            } else {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.gen_fail", "0" => err_msg).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_init(ctx: &CallbackContext) -> HandlerResult {
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::with_capacity(3);

    buttons.push(vec![
        InlineKeyboardButton::callback(t!("xray.kcp_cat_enc"), "u_kcp_cat:enc"),
        InlineKeyboardButton::callback(t!("xray.kcp_cat_obf"), "u_kcp_cat:obf"),
    ]);
    buttons.push(vec![
        InlineKeyboardButton::callback(t!("xray.kcp_cat_dis"), "u_kcp_cat:dis"),
        InlineKeyboardButton::callback(t!("xray.kcp_cat_ext"), "u_kcp_cat:ext"),
    ]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "m_xray_mgmt",
    )]);

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.kcp_title"))
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_cat(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let cat_code = data.strip_prefix("u_kcp_cat:").unwrap_or("enc");
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("unknown");

    let variants = KcpMask::variants_by_category(cat_code);
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::with_capacity(variants.len());

    for mask in &variants {
        buttons.push(vec![InlineKeyboardButton::callback(
            format!("✅ {}", mask.display_name()),
            format!("u_kcp_add:{}", mask.code()),
        )]);
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_back_cat"),
        "u_kcp_init",
    )]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            format!(
                "{}\n\n{}",
                t!("xray.kcp_select_mask", "0" => cat_name),
                mask_list
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_add(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let code = data.strip_prefix("u_kcp_add:").unwrap_or("ml");
    if code == "rl" {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.kcp_realm_note"))
            .await?;
        let m = KcpMask::from_code(code).unwrap();
        let stack_display = format!("1️⃣ {}", m.display_name());
        let buttons = vec![
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_add_more"),
                format!("u_kcp_more:{}", code),
            )],
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_done_btn"),
                format!("u_kcp_done:{}", code),
            )],
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_clear_btn"),
                "u_kcp_init",
            )],
        ];
        ctx.bot
            .edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                format!(
                    "{}\n\n{}",
                    t!("xray.kcp_stack_more", "0" => stack_display),
                    t!("xray.kcp_realm_note")
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        return Ok(HandlerAction::Done);
    }
    if let Some(m) = KcpMask::from_code(code) {
        if let Err(e) = m.is_compatible_with(&[]) {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(format!("❌ {}", e))
                .await?;
            return Ok(HandlerAction::Done);
        }
        let stack_display = format!("1️⃣ {}", m.display_name());
        let buttons = vec![
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_add_more"),
                format!("u_kcp_more:{}", code),
            )],
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_done_btn"),
                format!("u_kcp_done:{}", code),
            )],
            vec![InlineKeyboardButton::callback(
                t!("xray.kcp_clear_btn"),
                "u_kcp_init",
            )],
        ];
        ctx.bot
            .edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                t!("xray.kcp_stack_more", "0" => stack_display),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_more(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let existing = data.strip_prefix("u_kcp_more:").unwrap_or("");
    let existing_codes: Vec<&str> = existing.split(',').collect();

    let stack_display: Vec<String> = existing_codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let has_sudoku = current_masks.iter().any(|m| m.is_sudoku());
    let has_encryption = current_masks.iter().any(|m| m.is_encryption());

    let cat_counts = [
        (
            "enc",
            "🔐 Encryption",
            KcpMask::variants_by_category("enc").len(),
        ),
        (
            "obf",
            "🌀 Obfuscation",
            KcpMask::variants_by_category("obf").len(),
        ),
        (
            "ext",
            "⚡ Extension",
            KcpMask::variants_by_category("ext").len(),
        ),
    ];

    for (code, name, total) in &cat_counts {
        let added_count = existing_codes
            .iter()
            .filter(|ec| {
                KcpMask::from_code(ec)
                    .map(|m| m.category_code() == *code)
                    .unwrap_or(false)
            })
            .count();
        let remaining = total - added_count;

        let disabled_reason = match *code {
            "enc" if has_encryption => Some("added"),
            "obf" if has_sudoku => Some("sudoku added"),
            _ => None,
        };

        if let Some(reason) = disabled_reason {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("⛔ {} ({})", name, reason),
                "noop",
            )]);
        } else if remaining > 0 {
            if buttons.is_empty() || buttons.last().unwrap().len() >= 2 {
                buttons.push(Vec::new());
            }
            buttons
                .last_mut()
                .unwrap()
                .push(InlineKeyboardButton::callback(
                    format!("{} ({})", name, remaining),
                    format!("u_kcp_mcat:{}:{}", existing, code),
                ));
        } else {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("⛔ {} (max reached)", name),
                "noop",
            )]);
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_done_btn"),
        format!("u_kcp_done:{}", existing),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_clear_btn"),
        "u_kcp_init",
    )]);

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.kcp_select_cat_stack", "0" => stack_display.join("\n"), "1" => existing_codes.len()),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_mcat(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let rest = data.strip_prefix("u_kcp_mcat:").unwrap_or("");
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let existing = parts[0];
    let cat_code = parts[1];
    let existing_codes: Vec<&str> = existing.split(',').collect();
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("unknown");

    let variants = KcpMask::variants_by_category(cat_code);

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let stack_display: Vec<String> = existing_codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::with_capacity(variants.len());

    for mask in &variants {
        let code = mask.code();
        if existing_codes.contains(&code) {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("☑️ {}", mask.display_name()),
                "noop",
            )]);
        } else {
            match mask.is_compatible_with(&current_masks) {
                Ok(()) => {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("✅ {}", mask.display_name()),
                        format!("u_kcp_push:{}:{}", existing, code),
                    )]);
                }
                Err(e) => {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("⛔ {} ({})", mask.display_name(), e),
                        format!("noop:⛔:{}", code),
                    )]);
                }
            }
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_back_cat"),
        format!("u_kcp_more:{}", existing),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_done_btn"),
        format!("u_kcp_done:{}", existing),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_clear_btn"),
        "u_kcp_init",
    )]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            format!(
                "{}\n{}\n\n{}\n\n{}",
                t!("xray.kcp_current_stack"),
                stack_display.join("\n"),
                t!("xray.kcp_select_mask", "0" => cat_name),
                mask_list
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_push(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let rest = data.strip_prefix("u_kcp_push:").unwrap_or("");
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let existing = parts[0];
    let new_code = parts[1];

    let existing_codes: Vec<&str> = if existing.is_empty() {
        vec![]
    } else {
        existing.split(',').collect()
    };

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let new_mask = match KcpMask::from_code(new_code) {
        Some(m) => m,
        None => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.kcp_unknown_type"))
                .await?;
            return Ok(HandlerAction::Done);
        }
    };

    if let Err(e) = new_mask.is_compatible_with(&current_masks) {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(format!("❌ {}", e))
            .await?;
        return Ok(HandlerAction::Done);
    }

    let new_stack = if existing.is_empty() {
        new_code.to_string()
    } else {
        format!("{},{}", existing, new_code)
    };
    let codes: Vec<&str> = new_stack.split(',').collect();

    let stack_display: Vec<String> = codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let mut buttons = Vec::new();

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_add_more"),
        format!("u_kcp_more:{}", new_stack),
    )]);

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_done_btn"),
        format!("u_kcp_done:{}", new_stack),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.kcp_clear_btn"),
        "u_kcp_init",
    )]);

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.kcp_stack_more", "0" => stack_display.join("\n")),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_done(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let mask_codes_str = data.strip_prefix("u_kcp_done:").unwrap_or("");
    let codes: Vec<&str> = mask_codes_str.split(',').collect();

    if codes.is_empty() {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.kcp_min_one"))
            .await?;
        return Ok(HandlerAction::Done);
    }

    let masks: Vec<KcpMask> = codes.iter().filter_map(|c| KcpMask::from_code(c)).collect();

    let ordered = KcpMask::canonical_order(&masks);

    if let Err(e) = KcpMask::validate_stack(&ordered) {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(format!("❌ {}", e))
            .await?;
        return Ok(HandlerAction::Done);
    }

    let warnings = KcpMask::get_stack_warnings(&ordered);
    let stack_display: Vec<String> = ordered
        .iter()
        .map(|m| m.display_name().to_string())
        .collect();

    let ordered_codes: Vec<String> = ordered.iter().map(|m| m.code().to_string()).collect();
    let ordered_str = ordered_codes.join(",");

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        "🌐 IPv4 (0.0.0.0)",
        format!("u_kcp_ip:{}:4", ordered_str),
    )]];
    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("u_kcp_ip:{}:6", ordered_str),
        ));
    }
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.dual_v4"),
        format!("u_kcp_ip:{}:s4", ordered_str),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("xray.dual_v6"),
        format!("u_kcp_ip:{}:s6", ordered_str),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        format!("u_kcp_more:{}", mask_codes_str),
    )]);

    let warning_text = if warnings.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", warnings.join("\n"))
    };

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.kcp_stack_config", "0" => stack_display.join(" → "), "1" => warning_text, "2" => t!("xray.batch_step_ip")),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_ip(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let rest = data.strip_prefix("u_kcp_ip:").unwrap_or("");
    let last_colon = rest.rfind(':').unwrap_or(rest.len());
    let mask_codes_str = &rest[..last_colon];
    let ip_ver_code = &rest[last_colon + 1..];
    let codes: Vec<&str> = mask_codes_str.split(',').collect();

    let ip_version: IpVersion = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s4" => IpVersion::SplitStackV4Primary,
        "s6" => IpVersion::SplitStackV6Primary,
        _ => IpVersion::IPv4,
    };
    let ip_display: std::borrow::Cow<'_, str> = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV4Primary => t!("xray.dual_v4"),
        IpVersion::SplitStackV6Primary => t!("xray.dual_v6"),
    };

    let stack_display: Vec<String> = codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let buttons = vec![
        vec![
            InlineKeyboardButton::callback(
                "1",
                format!("u_kcp_ok:{}:{}:1", mask_codes_str, ip_ver_code),
            ),
            InlineKeyboardButton::callback(
                "3",
                format!("u_kcp_ok:{}:{}:3", mask_codes_str, ip_ver_code),
            ),
            InlineKeyboardButton::callback(
                "5",
                format!("u_kcp_ok:{}:{}:5", mask_codes_str, ip_ver_code),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                "10",
                format!("u_kcp_ok:{}:{}:10", mask_codes_str, ip_ver_code),
            ),
            InlineKeyboardButton::callback(
                "20",
                format!("u_kcp_ok:{}:{}:20", mask_codes_str, ip_ver_code),
            ),
            InlineKeyboardButton::callback(
                "50",
                format!("u_kcp_ok:{}:{}:50", mask_codes_str, ip_ver_code),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            format!("u_kcp_done:{}", mask_codes_str),
        )],
    ];

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("xray.kcp_batch_title", "0" => stack_display.join("\n"), "1" => ip_display, "2" => "⬇️ <b>Please select quantity:</b>"),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_kcp_ok(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let rest = data.strip_prefix("u_kcp_ok:").unwrap_or("");
    let parts: Vec<&str> = rest.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let n: usize = parts[0].parse().unwrap_or(0);
    let remaining = parts[1];
    let last_colon = remaining.rfind(':').unwrap_or(remaining.len());
    let mask_codes_str = &remaining[..last_colon];
    let ip_ver_code = &remaining[last_colon + 1..];

    let ip_version: IpVersion = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s4" => IpVersion::SplitStackV4Primary,
        "s6" => IpVersion::SplitStackV6Primary,
        _ => IpVersion::IPv4,
    };
    let ip_str: std::borrow::Cow<'_, str> = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV4Primary => t!("xray.dual_v4"),
        IpVersion::SplitStackV6Primary => t!("xray.dual_v6"),
    };

    let mask_codes: Vec<&str> = mask_codes_str.split(',').collect();

    let mask_names: Vec<&str> = mask_codes
        .iter()
        .filter_map(|c| KcpMask::from_code(c).map(|m| m.display_name()))
        .collect();
    let mask_label = mask_names.join("+");

    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("xray.gen_kcp_progress", "0" => n))
        .await?;

    let res = ConfigManager::batch_create_kcp(n, ip_version, &mask_codes).await;

    let adapter = ctx.state.adapter.clone();
    let target = TargetId(ctx.chat_id.0.to_string());

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.kcp_batch_done",
                "0" => result.created_count,
                "1" => ip_str,
                "2" => mask_label
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.kcp_config_file", "0" => filename)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: result_msg,
                        markup: None,
                    },
                )
                .await
            {
                message_ids.push(msg.0);
            }

            let adapter_clone = adapter.clone();
            let target_clone = target.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                for id_str in message_ids {
                    let mid = aegis::adapters::common::MessageId(id_str);
                    if let Err(e) = adapter_clone.delete_message(&target_clone, &mid).await {
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("xray.gen_fail", "0" => e).to_string(),
                        markup: None,
                    },
                )
                .await;
        }
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_user_list(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let idx: usize = data
        .strip_prefix("u_l:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let inbounds = ConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    if let Err(e) = utils::validate_idx(idx, inbounds.len(), &t!("xray.user_label")) {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(format!("❌ {}", e))
            .await?;
        return Ok(HandlerAction::Done);
    }
    if let Some(path) = inbounds.get(idx) {
        let clients = ConfigManager::get_clients_from_config(path)
            .await
            .unwrap_or_default();
        let mut buttons = Vec::new();
        for client in clients {
            let email = client["email"]
                .as_str()
                .or(client["name"].as_str())
                .unwrap_or("Unknown");
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("👤 {}", email),
                format!("u_d:{}:{}", idx, email),
            )]);
        }
        buttons.push(vec![InlineKeyboardButton::callback(
            t!("menu.back_user"),
            "m_usr",
        )]);
        ctx.bot
            .edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                t!("xray.user_list_title", "0" => path.split('/').next_back().unwrap_or("Unknown")),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_user_del(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let parts: Vec<&str> = data
        .strip_prefix("u_d:")
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() == 2 {
        let idx: usize = parts[0].parse().unwrap_or(0);
        let email = parts[1];
        let inbounds = ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default();

        if let Some(_path) = inbounds.get(idx) {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "⚠️ Confirm Delete",
                    format!("u_d_confirm:{}:{}", idx, email),
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back"),
                    format!("u_l:{}", idx),
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("xray.user_del_confirm", "0" => utils::escape_html(email)),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        } else {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.user_cfg_not_found"))
                .await?;
        }
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_user_del_confirm(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let parts: Vec<&str> = data
        .strip_prefix("u_d_confirm:")
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() == 2 {
        let email = parts[1];
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("xray.user_del_not_supported", "0" => email))
            .show_alert(true)
            .await?;
    }

    Ok(HandlerAction::Done)
}
