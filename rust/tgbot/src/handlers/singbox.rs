use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::batch_handler::send_singbox_batch_result;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::core::types::IpVersion;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::system::SystemMonitor;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    let _lang = ctx.state.language().await;

    match data {
        "m_singbox_mgmt" => {
            let is_installed = SingBoxInstaller::is_installed().await;
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let mut buttons = Vec::new();

            if !is_installed {
                buttons.push(vec![InlineKeyboardButton::callback(
                    "🚀 安装 Sing-box",
                    "sb_install",
                )]);
                ctx.bot.edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未检测到 Sing-box</b>\n\n系统尚未安装 Sing-box，无法使用 Hysteria2/TUIC 协议。",
                )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else if inbounds.is_empty() {
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                    InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                ctx.bot.edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未找到配置文件</b>\n\n您可以创建 Hysteria2 或 TUIC 批量配置。",
                )
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
                    "🗑️ 删除管理",
                    "sb_del_cfg",
                )]);
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                    InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        "📦 <b>Sing-box 管理</b>\n选择配置文件:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }

            Ok(HandlerAction::Done)
        }

        "sb_install" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⏳ 正在安装 Sing-box...")
                .await?;

            let bot_clone = ctx.bot.clone();
            let chat_id = ctx.chat_id;
            tokio::spawn(async move {
                match SingBoxInstaller::install().await {
                    Ok(_) => {
                        let _ = bot_clone.send_message(chat_id, "✅ <b>Sing-box 安装成功！</b>\n\n现在您可以创建 Hysteria2 或 TUIC 配置了。").parse_mode(ParseMode::Html).await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e))
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
                "⬅️ 返回",
                "m_singbox_mgmt",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🚀 <b>Hysteria2 批量创建</b>\n\n请选择网络协议版本:",
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
                "⬅️ 返回",
                "m_singbox_mgmt",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🚀 <b>TUIC 批量创建</b>\n\n请选择网络协议版本:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_ip:") => {
            let ip_ver = d.strip_prefix("sb_h2_ip:").unwrap_or("4");
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
                vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_h2_init")],
            ];

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "🚀 <b>Hysteria2 批量创建</b>\n\n🌐 网络协议版本: {}\n\n请选择生成数量:",
                        if ip_ver == "4" { "IPv4" } else { "IPv6" }
                    ),
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
                    .text("参数错误")
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };

            let buttons = vec![
                vec![InlineKeyboardButton::callback(
                    "🟢 推荐：开启混淆",
                    format!("sb_h2_exec:{}:{}:1", ip_ver, count),
                )],
                vec![InlineKeyboardButton::callback(
                    "🔴 不开启",
                    format!("sb_h2_exec:{}:{}:0", ip_ver, count),
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_h2_init")],
            ];

            ctx.bot.edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                format!(
                    "🚀 <b>Hysteria2 批量创建</b>\n\n\
                    🌐 网络协议: {}\n\
                    📊 生成数量: {}\n\n\
                    ⚠️ <b>提示</b>：如果您的运营商针对 QUIC 流量进行限制或干扰，建议开启 Salamander 混淆\n\n\
                    是否启用混淆?",
                    ip_display, count
                ),
            )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tu_ip:") => {
            let ip_ver = d.strip_prefix("sb_tu_ip:").unwrap_or("4");
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
                vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_tu_init")],
            ];

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "🚀 <b>TUIC 批量创建</b>\n\n🌐 网络版本: {}\n\n请选择生成数量:",
                        if ip_ver == "4" { "IPv4" } else { "IPv6" }
                    ),
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
                    .text("参数错误")
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
                .text("⏳ 正在创建配置...")
                .await?;

            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_hysteria2(count, ip_version, obfs_enabled)
                    .await
                {
                    Ok(result) => {
                        if let Err(e) = send_singbox_batch_result(
                            &bot_clone,
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
                        let _ = bot_clone
                            .send_message(chat_id_clone, format!("❌ <b>创建失败</b>\n原因: {}", e))
                            .parse_mode(ParseMode::Html)
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
                    .text("参数错误")
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
                .text("⏳ 正在创建配置...")
                .await?;

            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_tuic(count, ip_version).await {
                    Ok(result) => {
                        if let Err(e) =
                            send_singbox_batch_result(&bot_clone, chat_id_clone, "TUIC", &result)
                                .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, format!("❌ <b>创建失败</b>\n原因: {}", e))
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        "sb_del_cfg" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "🧨 删除全部配置",
                    "sb_del_all_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    "➗ 按数量删除配置",
                    "sb_del_count",
                )],
                vec![InlineKeyboardButton::callback(
                    "🎯 指定配置删除",
                    "sb_del_select",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "m_singbox_mgmt")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🗑️ <b>Sing-box 删除管理</b>\n请选择删除方式 (操作不可逆):",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_del_all_confirm" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "⚠️ 确认清空所有配置 (不可恢复) ⚠️",
                    "sb_del_all_exec",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 取消", "sb_del_cfg")],
            ]);
            ctx.bot.edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                "🚨 <b>二次确认</b>\n您确定要删除 <b>所有</b> Sing-box 配置文件吗？\n此操作将清空所有配置文件、重启 Sing-box 并清理端口跳跃规则。",
            )
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
                        .text(format!("✅ 已彻底清空 {} 个 Sing-box 配置文件", count))
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(format!("❌ 删除失败: {}", e))
                        .show_alert(true)
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("sb_del_cfg".to_string()))
        }

        "sb_del_count" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("10 个", "sb_del_exec_count:10"),
                    InlineKeyboardButton::callback("50 个", "sb_del_exec_count:50"),
                ],
                vec![
                    InlineKeyboardButton::callback("100 个", "sb_del_exec_count:100"),
                    InlineKeyboardButton::callback("500 个", "sb_del_exec_count:500"),
                ],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_del_cfg")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "➗ <b>Sing-box 按数量删除 (由旧到新)</b>\n请选择要删除的文件数量:",
                )
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
                        .text(format!("✅ 已删除 {} 个最旧的配置文件", deleted))
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(format!("❌ 删除失败: {}", e))
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
                    .text("⚠️ 没有可删除的配置文件")
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
                    "⬅️ 返回",
                    "sb_del_cfg",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!(
                            "🎯 <b>Sing-box 指定配置删除</b>\n\n共 {} 个配置文件，请选择要删除的:",
                            count
                        ),
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
                            .text(format!("✅ 已删除配置文件: {}", filename))
                            .show_alert(true)
                            .await?;
                    }
                    Err(e) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(format!("❌ 删除失败: {}", e))
                            .show_alert(true)
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("❌ 文件索引无效")
                    .show_alert(true)
                    .await?;
            }
            Ok(HandlerAction::Redirect("sb_del_select".to_string()))
        }

        _ => Ok(HandlerAction::Done),
    }
}
