use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::batch_handler::send_singbox_batch_result;
use aegis::adapters::common::{MessageContent, TargetId};
use aegis::core::singbox::{SingBoxConfigManager, SingBoxInstaller};
use aegis::core::system::SystemMonitor;
use aegis::core::types::IpVersion;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();

    match data {
        "m_singbox_mgmt" => {
            let is_installed = SingBoxInstaller::is_installed().await;
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let mut buttons = Vec::new();

            if !is_installed {
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_install"),
                    "sb_install",
                )]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_not_installed"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else if inbounds.is_empty() {
                buttons.push(vec![
                    InlineKeyboardButton::callback(t!("menu.singbox_h2_batch"), "sb_h2_init"),
                    InlineKeyboardButton::callback(t!("menu.singbox_tu_batch"), "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "m_usr",
                )]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_no_config"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else {
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("📁 {}", filename),
                        format!("sb_l:{}", i),
                    )]);
                }
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_delete_mgmt"),
                    "sb_del_cfg",
                )]);
                buttons.push(vec![
                    InlineKeyboardButton::callback(t!("menu.singbox_h2_batch"), "sb_h2_init"),
                    InlineKeyboardButton::callback(t!("menu.singbox_tu_batch"), "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "m_usr",
                )]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_mgmt_select"))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }

            Ok(HandlerAction::Done)
        }

        "sb_install" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("menu.singbox_installing"))
                .await?;

            let bot_clone = ctx.bot.clone();
            let chat_id = ctx.chat_id;
            tokio::spawn(async move {
                match SingBoxInstaller::install().await {
                    Ok(_) => {
                        let _ = bot_clone
                            .send_message(chat_id, t!("menu.singbox_install_success"))
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id,
                                t!("menu.singbox_install_fail", "0" => e.to_string()),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        "sb_h2_init" => {
            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            let mut buttons = vec![vec![InlineKeyboardButton::callback(
                "🌐 IPv4",
                "sb_h2_ip:4",
            )]];
            if has_ipv6 {
                buttons[0].push(InlineKeyboardButton::callback("🌐 IPv6", "sb_h2_ip:6"));
            }
            buttons.push(vec![InlineKeyboardButton::callback(
                t!("menu.back_user"),
                "m_singbox_mgmt",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "{}\n\n{}",
                        t!("menu.singbox_h2_batch_title"),
                        t!("menu.singbox_h2_batch_ip")
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_tu_init" => {
            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            let mut buttons = vec![vec![InlineKeyboardButton::callback(
                "🌐 IPv4",
                "sb_tu_ip:4",
            )]];
            if has_ipv6 {
                buttons[0].push(InlineKeyboardButton::callback("🌐 IPv6", "sb_tu_ip:6"));
            }
            buttons.push(vec![InlineKeyboardButton::callback(
                t!("menu.back_user"),
                "m_singbox_mgmt",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "{}\n\n{}",
                        t!("menu.singbox_tu_batch_title"),
                        t!("menu.singbox_tu_batch_ip")
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_ip:") => {
            let ip_ver = d.strip_prefix("sb_h2_ip:").unwrap_or("4");
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
            let buttons = vec![
                vec![
                    InlineKeyboardButton::callback("1", format!("sb_h2_obfs:{}:1", ip_ver)),
                    InlineKeyboardButton::callback("3", format!("sb_h2_obfs:{}:3", ip_ver)),
                    InlineKeyboardButton::callback("5", format!("sb_h2_obfs:{}:5", ip_ver)),
                ],
                vec![
                    InlineKeyboardButton::callback("10", format!("sb_h2_obfs:{}:10", ip_ver)),
                    InlineKeyboardButton::callback("20", format!("sb_h2_obfs:{}:20", ip_ver)),
                    InlineKeyboardButton::callback("50", format!("sb_h2_obfs:{}:50", ip_ver)),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "sb_h2_init",
                )],
            ];

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("menu.singbox_h2_qty_title", "0" => ip_display),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_obfs:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_obfs:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("menu.singbox_param_error"))
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };

            let buttons = vec![
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_h2_obfs_enable"),
                    format!("sb_h2_exec:{}:{}:1", ip_ver, count),
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_h2_obfs_disable"),
                    format!("sb_h2_exec:{}:{}:0", ip_ver, count),
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "sb_h2_init",
                )],
            ];

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("menu.singbox_h2_obfs_title", "0" => ip_display, "1" => count),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tu_ip:") => {
            let ip_ver = d.strip_prefix("sb_tu_ip:").unwrap_or("4");
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
            let buttons = vec![
                vec![
                    InlineKeyboardButton::callback("1", format!("sb_tu_exec:{}:1", ip_ver)),
                    InlineKeyboardButton::callback("3", format!("sb_tu_exec:{}:3", ip_ver)),
                    InlineKeyboardButton::callback("5", format!("sb_tu_exec:{}:5", ip_ver)),
                ],
                vec![
                    InlineKeyboardButton::callback("10", format!("sb_tu_exec:{}:10", ip_ver)),
                    InlineKeyboardButton::callback("20", format!("sb_tu_exec:{}:20", ip_ver)),
                    InlineKeyboardButton::callback("50", format!("sb_tu_exec:{}:50", ip_ver)),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "sb_tu_init",
                )],
            ];

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("menu.singbox_tu_qty_title", "0" => ip_display),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_exec:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_exec:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 3 {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("menu.singbox_param_error"))
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count: usize = parts[1].parse().unwrap_or(1);
            let obfs_enabled: bool = parts[2] == "1";
            let ip_version = if ip_ver == "6" {
                IpVersion::IPv6
            } else {
                IpVersion::IPv4
            };

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("menu.singbox_creating"))
                .await?;

            let adapter = ctx.state.adapter.clone();
            let chat_id_clone = ctx.chat_id;

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_hysteria2(count, ip_version, obfs_enabled)
                    .await
                {
                    Ok(result) => {
                        if let Err(e) = send_singbox_batch_result(
                            adapter.clone(),
                            chat_id_clone,
                            "Hysteria2",
                            &result,
                        )
                        .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
                    Err(e) => {
                        let target = TargetId(chat_id_clone.0.to_string());
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("menu.singbox_create_fail", "0" => e.to_string())
                                        .to_string(),
                                    markup: None,
                                },
                            )
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tu_exec:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_tu_exec:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("menu.singbox_param_error"))
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count: usize = parts[1].parse().unwrap_or(1);
            let ip_version = if ip_ver == "6" {
                IpVersion::IPv6
            } else {
                IpVersion::IPv4
            };

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("menu.singbox_creating"))
                .await?;

            let adapter = ctx.state.adapter.clone();
            let chat_id_clone = ctx.chat_id;

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_tuic(count, ip_version).await {
                    Ok(result) => {
                        if let Err(e) = send_singbox_batch_result(
                            adapter.clone(),
                            chat_id_clone,
                            "TUIC",
                            &result,
                        )
                        .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
                    Err(e) => {
                        let target = TargetId(chat_id_clone.0.to_string());
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("menu.singbox_create_fail", "0" => e.to_string())
                                        .to_string(),
                                    markup: None,
                                },
                            )
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        "sb_del_cfg" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_del_all"),
                    "sb_del_all_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_del_count"),
                    "sb_del_count",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_del_select"),
                    "sb_del_select",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "m_singbox_mgmt",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_del_title"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_del_all_confirm" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_confirm_clear"),
                    "sb_del_all_exec",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_cancel"),
                    "sb_del_cfg",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_confirm_del_all"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_del_all_exec" => {
            match SingBoxConfigManager::delete_all_configurations().await {
                Ok(count) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("menu.singbox_del_success_all", "0" => count.to_string()))
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("menu.singbox_del_fail", "0" => e.to_string()))
                        .show_alert(true)
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("sb_del_cfg".to_string()))
        }

        "sb_del_count" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("menu.singbox_count_10"),
                        "sb_del_exec_count:10",
                    ),
                    InlineKeyboardButton::callback(
                        t!("menu.singbox_count_50"),
                        "sb_del_exec_count:50",
                    ),
                ],
                vec![
                    InlineKeyboardButton::callback(
                        t!("menu.singbox_count_100"),
                        "sb_del_exec_count:100",
                    ),
                    InlineKeyboardButton::callback(
                        t!("menu.singbox_count_500"),
                        "sb_del_exec_count:500",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "sb_del_cfg",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_del_count_title"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_del_exec_count:") => {
            let n: usize = d
                .strip_prefix("sb_del_exec_count:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            match SingBoxConfigManager::delete_by_count(n).await {
                Ok(deleted) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("menu.singbox_del_success_count", "0" => deleted.to_string()))
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("menu.singbox_del_fail", "0" => e.to_string()))
                        .show_alert(true)
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("sb_del_cfg".to_string()))
        }

        "sb_del_select" => {
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let count = SingBoxConfigManager::get_config_count().await.unwrap_or(0);

            if inbounds.is_empty() {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("menu.singbox_no_files"))
                    .show_alert(true)
                    .await?;
            } else {
                let mut buttons = Vec::new();
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("🗑️ {}", filename),
                        format!("sb_del_file:{}", i),
                    )]);
                }
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.back_user"),
                    "sb_del_cfg",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("menu.singbox_del_select_title", "0" => count.to_string()),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_del_file:") => {
            let index: usize = d
                .strip_prefix("sb_del_file:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();

            if let Some(path) = inbounds.get(index) {
                match SingBoxConfigManager::delete_specific_configuration(path).await {
                    Ok(()) => {
                        let filename = path.split('/').next_back().unwrap_or("Unknown");
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("menu.singbox_del_success_specific", "0" => filename))
                            .show_alert(true)
                            .await?;
                    }
                    Err(e) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("menu.singbox_del_fail", "0" => e.to_string()))
                            .show_alert(true)
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("menu.singbox_invalid_index"))
                    .show_alert(true)
                    .await?;
            }
            Ok(HandlerAction::Redirect("sb_del_select".to_string()))
        }

        _ => Ok(HandlerAction::Done),
    }
}
