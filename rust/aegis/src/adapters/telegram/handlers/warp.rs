use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::utils;
use aegis::core::xray::installer::WarpInstaller;
use aegis::core::xray::{ConfigManager, WarpMode};
use rust_i18n::t;
use sha2::{Digest, Sha256};
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();

    match data {
        "m_warp" => {
            let is_installed = WarpInstaller::is_installed().await;
            if !is_installed {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        t!("warp.install_warp"),
                        "a_inst_warp",
                    )],
                    vec![InlineKeyboardButton::callback(
                        t!("menu.back_net_opt"),
                        "m_net_opt",
                    )],
                ]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.not_installed"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));

            let rule_display = if current_rules.is_empty() {
                t!("warp.no_rules").to_string()
            } else {
                let escaped_rules: Vec<String> = current_rules
                    .iter()
                    .map(|r| utils::escape_html(r))
                    .collect();
                if escaped_rules.len() > 5 {
                    format!(
                        "{} ({} {})",
                        escaped_rules
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", "),
                        t!("warp.total_count_prefix"),
                        escaped_rules.len()
                    )
                } else {
                    escaped_rules.join(", ")
                }
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("warp.add_rule"), "a_warp_add_input"),
                    InlineKeyboardButton::callback(t!("warp.del_rule"), "a_warp_del_menu"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("warp.mode_label", "0" => current_mode.as_str()),
                    "a_warp_switch_mode",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("warp.status_check"),
                    "a_warp_status",
                )],
                vec![
                    InlineKeyboardButton::callback(t!("warp.restart_service"), "a_warp_restart"),
                    InlineKeyboardButton::callback(
                        t!("warp.uninstall_service"),
                        "a_warp_uninstall",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("warp.clear_all"),
                    "a_warp_clear_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_net_opt"),
                    "m_net_opt",
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("warp.title", "0" => current_mode.as_str(), "1" => rule_display),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_switch_mode" => {
            let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));
            let next_mode = current_mode.next();

            match ConfigManager::update_warp_routing_rules(current_rules, next_mode).await {
                Ok(_) => Ok(HandlerAction::Redirect("m_warp".to_string())),
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("warp.mode_switch_fail", "0" => e.to_string()))
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_inst_warp" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("warp.installing"))
                .await?;
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.installing"))
                .parse_mode(ParseMode::Html)
                .await?;

            match WarpInstaller::install().await {
                Ok(_) => {
                    ctx.bot
                        .send_message(ctx.chat_id, t!("warp.install_success"))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, t!("warp.install_fail", "0" => e.to_string()))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_warp_add_input" => {
            ctx.state
                .start_warp_input(ctx.chat_id.0.to_string(), Instant::now())
                .await;
            ctx.bot
                .send_message(ctx.chat_id, t!("warp.add_input"))
                .parse_mode(ParseMode::Html)
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_del_menu" => {
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));

            if current_rules.is_empty() {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("warp.no_rules_del"))
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let mut buttons = Vec::new();
            for rule in current_rules.iter() {
                let mut hasher = Sha256::new();
                hasher.update(rule.as_bytes());
                let hash = hex::encode(hasher.finalize());
                let short_hash = &hash[..8];

                let display_rule = if rule.len() > 30 {
                    format!("{}...", utils::escape_html(&rule[..27]))
                } else {
                    utils::escape_html(rule)
                };

                buttons.push(vec![InlineKeyboardButton::callback(
                    format!("🗑 {}", display_rule),
                    format!("a_warp_del:{}", short_hash),
                )]);
            }
            buttons.push(vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                "m_warp",
            )]);

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.del_title"))
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            Ok(HandlerAction::Done)
        }
        d if d.starts_with("a_warp_del:") => {
            let hash_prefix = d.strip_prefix("a_warp_del:").unwrap_or("");
            if let Err(e) = utils::validate_hash_prefix(hash_prefix) {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or_default();

            let rule_to_delete = current_rules.iter().find(|r| {
                let mut hasher = Sha256::new();
                hasher.update(r.as_bytes());
                let hash = hex::encode(hasher.finalize());
                &hash[..8] == hash_prefix
            });

            if let Some(rule) = rule_to_delete {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        t!("warp.confirm_del"),
                        format!("a_warp_del_confirm:{}", hash_prefix),
                    )],
                    vec![InlineKeyboardButton::callback(
                        t!("warp.cancel_del"),
                        "a_warp_del_menu",
                    )],
                ]);

                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("warp.del_confirm", "0" => utils::escape_html(rule)),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("warp.rule_not_found"))
                    .await?;
                return Ok(HandlerAction::Redirect("a_warp_del_menu".to_string()));
            }
            Ok(HandlerAction::Done)
        }
        d if d.starts_with("a_warp_del_confirm:") => {
            let hash_prefix = d.strip_prefix("a_warp_del_confirm:").unwrap_or("");
            if let Err(e) = utils::validate_hash_prefix(hash_prefix) {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or_default();

            let rule_to_delete = current_rules.into_iter().find(|r| {
                let mut hasher = Sha256::new();
                hasher.update(r.as_bytes());
                let hash = hex::encode(hasher.finalize());
                &hash[..8] == hash_prefix
            });

            if let Some(rule) = rule_to_delete {
                match ConfigManager::remove_warp_routing_rule(&rule).await {
                    Ok(_) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("warp.rule_deleted"))
                            .show_alert(true)
                            .await?;
                    }
                    Err(e) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("warp.del_fail", "0" => e.to_string()))
                            .show_alert(true)
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("warp.rule_not_found"))
                    .show_alert(true)
                    .await?;
            }
            Ok(HandlerAction::Redirect("a_warp_del_menu".to_string()))
        }
        "a_warp_clear_confirm" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("warp.confirm_clear"),
                    "a_warp_clear_exec",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("warp.cancel_del"),
                    "m_warp",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.clear_confirm"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_clear_exec" => {
            match ConfigManager::update_warp_routing_rules(Vec::new(), WarpMode::Default).await {
                Ok(_) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("warp.all_cleared"))
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("warp.clear_fail", "0" => e.to_string()))
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_warp_status" => match WarpInstaller::status().await {
            Ok(status) => {
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!("📊 <b>WARP {}</b>\n\n{}", t!("warp.status_label"), status),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback(t!("menu.back"), "m_warp"),
                    ]]))
                    .await?;
                Ok(HandlerAction::Done)
            }
            Err(e) => {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("warp.status_fail", "0" => e.to_string()))
                    .await?;
                Ok(HandlerAction::Done)
            }
        },
        "a_warp_restart" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("warp.restarting"))
                .await?;
            match WarpInstaller::restart_service().await {
                Ok(_) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("warp.restart_success"))
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, t!("warp.restart_fail", "0" => e.to_string()))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("warp.confirm_uninstall"),
                    "a_warp_uninstall_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("warp.cancel_del"),
                    "m_warp",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.uninstall_confirm"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall_confirm" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("warp.uninstalling"))
                .await?;
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("warp.uninstalling"))
                .parse_mode(ParseMode::Html)
                .await?;

            match WarpInstaller::uninstall().await {
                Ok(_) => {
                    ctx.bot
                        .send_message(ctx.chat_id, t!("warp.uninstall_success"))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, t!("warp.uninstall_fail", "0" => e.to_string()))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        _ => Ok(HandlerAction::Done),
    }
}
