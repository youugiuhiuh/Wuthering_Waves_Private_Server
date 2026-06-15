use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::utils;
use sha2::{Digest, Sha256};
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use aegis::core::xray::{ConfigManager, WarpMode};
use aegis::core::xray::installer::WarpInstaller;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();

    match data {
        "m_warp" => {
            let is_installed = WarpInstaller::is_installed().await;
            if !is_installed {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "🚀 安装 Cloudflare WARP",
                        "a_inst_warp",
                    )],
                    vec![InlineKeyboardButton::callback(
                        "⬅️ 返回网络优化",
                        "m_net_opt",
                    )],
                ]);
                ctx.bot.edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "⚠️ <b>未检测到 Cloudflare WARP</b>\n\n系统未安装 WARP 服务，无法配置分流规则。\n是否立即安装？",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
                return Ok(HandlerAction::Done);
            }

            let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));

            let rule_display = if current_rules.is_empty() {
                "<i>(无规则)</i>".to_string()
            } else {
                let escaped_rules: Vec<String> = current_rules
                    .iter()
                    .map(|r| utils::escape_html(r))
                    .collect();
                if escaped_rules.len() > 5 {
                    format!(
                        "{} (共 {} 条)",
                        escaped_rules
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", "),
                        escaped_rules.len()
                    )
                } else {
                    escaped_rules.join(", ")
                }
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("➕ 添加规则", "a_warp_add_input"),
                    InlineKeyboardButton::callback("➖ 删除规则", "a_warp_del_menu"),
                ],
                vec![InlineKeyboardButton::callback(
                    format!("⚙️ 模式: {}", current_mode.as_str()),
                    "a_warp_switch_mode",
                )],
                vec![InlineKeyboardButton::callback(
                    "📊 状态检测",
                    "a_warp_status",
                )],
                vec![
                    InlineKeyboardButton::callback("🔄 重启服务", "a_warp_restart"),
                    InlineKeyboardButton::callback("🗑️ 卸载服务", "a_warp_uninstall"),
                ],
                vec![InlineKeyboardButton::callback(
                    "🗑️ 清空所有规则",
                    "a_warp_clear_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回网络优化",
                    "m_net_opt",
                )],
            ]);

            ctx.bot.edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                format!("🌩 <b>WARP 分流管理</b>\n\n当前模式: <b>{}</b>\n当前规则: {}\n\n您可以添加或删除特定的域名/GeoSite规则。", current_mode.as_str(), rule_display)
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
                        .text(format!("❌ 切换失败: {}", e))
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_inst_warp" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⏳ 正在安装 Cloudflare WARP...")
                .await?;
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "⏳ <b>正在安装 Cloudflare WARP...</b>\n请稍候，这可能需要几分钟。",
                )
                .parse_mode(ParseMode::Html)
                .await?;

            match WarpInstaller::install().await {
                Ok(_) => {
                    ctx.bot
                        .send_message(
                            ctx.chat_id,
                            "✅ <b>Cloudflare WARP 安装成功！</b>\n现在您可以配置分流规则了。",
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_warp_add_input" => {
            ctx.state
                .start_warp_input(ctx.chat_id, Instant::now())
                .await;
            ctx.bot.send_message(
                ctx.chat_id,
                "✏️ <b>请输入要添加的分流规则</b>\n\n支持格式: `geosite:google, domain:reddit.com`\n多个规则请用逗号或换行分隔。\n\n(输入将在 60 秒后超时)",
            )
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
                    .text("⚠️ 暂无规则可删除")
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
            buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_warp")]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "➖ <b>删除规则</b>\n点击以删除对应规则:",
                )
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
                        "⚠️ 确认删除",
                        format!("a_warp_del_confirm:{}", hash_prefix),
                    )],
                    vec![InlineKeyboardButton::callback("🔙 取消", "a_warp_del_menu")],
                ]);

                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!(
                            "⚠️ <b>删除确认</b>\n\n您确定要删除分流规则 <code>{}</code> 吗？",
                            utils::escape_html(rule)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("❌ 规则未找到")
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
                            .text("✅ 规则已删除")
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
                    .text("❌ 规则未找到")
                    .show_alert(true)
                    .await?;
            }
            Ok(HandlerAction::Redirect("a_warp_del_menu".to_string()))
        }
        "a_warp_clear_confirm" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "⚠️ 确认清空",
                    "a_warp_clear_exec",
                )],
                vec![InlineKeyboardButton::callback("🔙 取消", "m_warp")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "⚠️ <b>清空确认</b>\n此操作将删除所有分流规则，且不可恢复。",
                )
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
                        .text("✅ 所有规则已清空")
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(format!("❌ 清空失败: {}", e))
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
                        format!("📊 <b>WARP 状态检测</b>\n\n{}", status),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback("⬅️ 返回", "m_warp"),
                    ]]))
                    .await?;
                Ok(HandlerAction::Done)
            }
            Err(e) => {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(format!("❌ 检测失败: {}", e))
                    .await?;
                Ok(HandlerAction::Done)
            }
        },
        "a_warp_restart" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⏳ 正在重启服务...")
                .await?;
            match WarpInstaller::restart_service().await {
                Ok(_) => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text("✅ 服务重启成功且连接正常")
                        .await?;
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, format!("❌ <b>重启失败</b>\n原因: {}", e))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "⚠️ 确认卸载",
                    "a_warp_uninstall_confirm",
                )],
                vec![InlineKeyboardButton::callback("🔙 取消", "m_warp")],
            ]);
            ctx.bot.edit_message_text(
                ctx.chat_id,
                ctx.msg_id,
                "⚠️ <b>卸载确认</b>\n\n确定要卸载 Cloudflare WARP 吗？\n这将移除所有相关组件和配置。",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall_confirm" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⏳ 正在卸载...")
                .await?;
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, "⏳ <b>正在卸载...</b>")
                .parse_mode(ParseMode::Html)
                .await?;

            match WarpInstaller::uninstall().await {
                Ok(_) => {
                    ctx.bot
                        .send_message(
                            ctx.chat_id,
                            "✅ <b>卸载成功</b>\nCloudflare WARP 已从系统中移除。",
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    ctx.bot
                        .send_message(ctx.chat_id, format!("❌ <b>卸载失败</b>\n原因: {}", e))
                        .parse_mode(ParseMode::Html)
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        _ => Ok(HandlerAction::Done),
    }
}
