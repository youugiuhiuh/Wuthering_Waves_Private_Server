//! WARP 分流管理模块

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use std::sync::Arc;
use std::time::Instant;
use sha2::{Digest, Sha256};

use crate::app::state::AppState;
use crate::logic::config::{ConfigManager, WarpMode};
use crate::logic::installer::WarpInstaller;
use tgbot::core::error::{Result, AppError};

/// HTML 转义
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn validate_hash_prefix(prefix: &str) -> Result<&str> {
    if prefix.is_empty() {
        return Err(AppError::InvalidParameter("hash 前缀不能为空".to_string()));
    }
    if prefix.len() > 8 {
        return Err(AppError::InvalidParameter(format!("hash 前缀过长: {} (最大 8)", prefix.len())));
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::InvalidParameter("hash 前缀包含无效字符".to_string()));
    }
    Ok(prefix)
}

/// WARP 主菜单
pub async fn handle_m_warp(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
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
        bot.edit_message_text(
            chat_id,
            msg_id,
            "⚠️ <b>未检测到 Cloudflare WARP</b>\n\n系统未安装 WARP 服务，无法配置分流规则。\n是否立即安装？",
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
        return Ok(());
    }

    let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
        .await
        .unwrap_or((Vec::new(), WarpMode::Default));

    let rule_display = if current_rules.is_empty() {
        "<i>(无规则)</i>".to_string()
    } else {
        let escaped_rules: Vec<String> =
            current_rules.iter().map(|r| escape_html(r)).collect();
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

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🌩 <b>WARP 分流管理</b>\n\n当前模式: <b>{}</b>\n当前规则: {}\n\n您可以添加或删除特定的域名/GeoSite规则。", current_mode.as_str(), rule_display)
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 切换 WARP 模式
pub async fn handle_a_warp_switch_mode(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
        .await
        .unwrap_or((Vec::new(), WarpMode::Default));
    let next_mode = current_mode.next();

    match ConfigManager::update_warp_routing_rules(current_rules, next_mode).await {
        Ok(_) => {
            let new_q = q.clone();
            q = CallbackQuery {
                data: Some("m_warp".to_string()),
                ..new_q
            };
        }
        Err(e) => {
            bot.answer_callback_query(q.id)
                .text(format!("❌ 切换失败: {}", e))
                .await?;
        }
    }
    Ok(())
}

/// 安装 WARP
pub async fn handle_a_inst_warp(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在安装 Cloudflare WARP...")
        .await?;
    bot.edit_message_text(
        chat_id,
        msg_id,
        "⏳ <b>正在安装 Cloudflare WARP...</b>\n请稍候，这可能需要几分钟。",
    )
    .parse_mode(ParseMode::Html)
    .await?;

    match WarpInstaller::install().await {
        Ok(_) => {
            bot.send_message(
                chat_id,
                "✅ <b>Cloudflare WARP 安装成功！</b>\n现在您可以配置分流规则了。",
            )
            .parse_mode(ParseMode::Html)
            .await?;

            let new_q = q.clone();
            q = CallbackQuery {
                data: Some("m_warp".to_string()),
                ..new_q
            };
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e))
                .parse_mode(ParseMode::Html)
                .await?;
        }
    }
    Ok(())
}

/// 添加 WARP 分流规则输入
pub async fn handle_a_warp_add_input(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    state.start_warp_input(chat_id, Instant::now()).await;
    bot.send_message(
        chat_id,
        "✏️ <b>请输入要添加的分流规则</b>\n\n支持格式: `geosite:google, domain:reddit.com`\n多个规则请用逗号或换行分隔。\n\n(输入将在 60 秒后超时)",
    )
    .parse_mode(ParseMode::Html)
    .await?;
    Ok(())
}

/// 删除规则菜单
pub async fn handle_a_warp_del_menu(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let (current_rules, _) = ConfigManager::get_warp_routing_rules()
        .await
        .unwrap_or((Vec::new(), WarpMode::Default));

    if current_rules.is_empty() {
        bot.answer_callback_query(q.id)
            .text("⚠️ 暂无规则可删除")
            .await?;
        return Ok(());
    }

    let mut buttons = Vec::new();
    for rule in current_rules.iter() {
        let mut hasher = Sha256::new();
        hasher.update(rule.as_bytes());
        let hash = hex::encode(hasher.finalize());
        let short_hash = &hash[..8];

        let display_rule = if rule.len() > 30 {
            format!("{}...", escape_html(&rule[..27]))
        } else {
            escape_html(rule)
        };

        buttons.push(vec![InlineKeyboardButton::callback(
            format!("🗑 {}", display_rule),
            format!("a_warp_del:{}", short_hash),
        )]);
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_warp")]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        "➖ <b>删除规则</b>\n点击以删除对应规则:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// 删除规则确认
pub async fn handle_a_warp_del(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let hash_prefix = data.strip_prefix("a_warp_del:").unwrap_or("");
    if let Err(e) = validate_hash_prefix(hash_prefix) {
        bot.answer_callback_query(q.id.clone())
            .text(&format!("❌ {}", e))
            .await?;
        return Ok(());
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

        bot.edit_message_text(
            chat_id,
            msg_id,
            format!(
                "⚠️ <b>删除确认</b>\n\n您确定要删除分流规则 <code>{}</code> 吗？",
                escape_html(rule)
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    } else {
        bot.answer_callback_query(q.id.clone())
            .text("❌ 规则未找到")
            .await?;
    }
    Ok(())
}

/// 执行删除规则
pub async fn handle_a_warp_del_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let hash_prefix = data.strip_prefix("a_warp_del_confirm:").unwrap_or("");
    if let Err(e) = validate_hash_prefix(hash_prefix) {
        bot.answer_callback_query(q.id.clone())
            .text(&format!("❌ {}", e))
            .await?;
        return Ok(());
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
                bot.answer_callback_query(q.id.clone())
                    .text("✅ 规则已删除")
                    .show_alert(true)
                    .await?;
            }
            Err(e) => {
                bot.answer_callback_query(q.id.clone())
                    .text(format!("❌ 删除失败: {}", e))
                    .show_alert(true)
                    .await?;
            }
        }
    } else {
        bot.answer_callback_query(q.id.clone())
            .text("❌ 规则未找到")
            .show_alert(true)
            .await?;
    }
    Ok(())
}

/// 清空所有规则确认
pub async fn handle_a_warp_clear_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "⚠️ 确认清空",
            "a_warp_clear_exec",
        )],
        vec![InlineKeyboardButton::callback("🔙 取消", "m_warp")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        "⚠️ <b>清空确认</b>\n此操作将删除所有分流规则，且不可恢复。",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行清空所有规则
pub async fn handle_a_warp_clear_exec(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    match ConfigManager::update_warp_routing_rules(Vec::new(), WarpMode::Default)
        .await
    {
        Ok(_) => {
            bot.answer_callback_query(q.id.clone())
                .text("✅ 所有规则已清空")
                .await?;
            let new_q = q.clone();
            q = CallbackQuery {
                data: Some("m_warp".to_string()),
                ..new_q
            };
        }
        Err(e) => {
            bot.answer_callback_query(q.id)
                .text(format!("❌ 清空失败: {}", e))
                .await?;
        }
    }
    Ok(())
}

/// WARP 状态检测
pub async fn handle_a_warp_status(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    match WarpInstaller::status().await {
        Ok(status) => {
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!("📊 <b>WARP 状态检测</b>\n\n{}", status),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("⬅️ 返回", "m_warp"),
            ]]))
            .await?;
        }
        Err(e) => {
            bot.answer_callback_query(q.id)
                .text(format!("❌ 检测失败: {}", e))
                .await?;
        }
    }
    Ok(())
}

/// 重启 WARP 服务
pub async fn handle_a_warp_restart(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在重启服务...")
        .await?;
    match WarpInstaller::restart_service().await {
        Ok(_) => {
            bot.answer_callback_query(q.id)
                .text("✅ 服务重启成功且连接正常")
                .await?;
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ <b>重启失败</b>\n原因: {}", e))
                .parse_mode(ParseMode::Html)
                .await?;
        }
    }
    Ok(())
}

/// 卸载 WARP 确认
pub async fn handle_a_warp_uninstall(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "⚠️ 确认卸载",
            "a_warp_uninstall_confirm",
        )],
        vec![InlineKeyboardButton::callback("🔙 取消", "m_warp")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        "⚠️ <b>卸载确认</b>\n\n确定要卸载 Cloudflare WARP 吗？\n这将移除所有相关组件和配置。",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行卸载 WARP
pub async fn handle_a_warp_uninstall_confirm(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在卸载...")
        .await?;
    bot.edit_message_text(chat_id, msg_id, "⏳ <b>正在卸载...</b>")
        .parse_mode(ParseMode::Html)
        .await?;

    match WarpInstaller::uninstall().await {
        Ok(_) => {
            bot.send_message(
                chat_id,
                "✅ <b>卸载成功</b>\nCloudflare WARP 已从系统中移除。",
            )
            .parse_mode(ParseMode::Html)
            .await?;

            let new_q = q.clone();
            q = CallbackQuery {
                data: Some("m_warp".to_string()),
                ..new_q
            };
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ <b>卸载失败</b>\n原因: {}", e))
                .parse_mode(ParseMode::Html)
                .await?;
        }
    }
    Ok(())
}

/// WARP 回调分派
pub async fn dispatch_callback(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: &Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        "m_warp" => handle_m_warp(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "a_warp_switch_mode" => {
            handle_a_warp_switch_mode(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        "a_inst_warp" => handle_a_inst_warp(bot.clone(), q.clone(), chat_id, msg_id).await?,
        d if d.starts_with("a_warp_add_input:") => {
            handle_a_warp_add_input(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?
        }
        "a_warp_del_menu" => {
            handle_a_warp_del_menu(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        d if d.starts_with("a_warp_del:") => {
            handle_a_warp_del(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("a_warp_del_confirm:") => {
            handle_a_warp_del_confirm(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "a_warp_clear_confirm" => {
            handle_a_warp_clear_confirm(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        "a_warp_clear_exec" => {
            handle_a_warp_clear_exec(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        "a_warp_status" => handle_a_warp_status(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "a_warp_restart" => handle_a_warp_restart(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "a_warp_uninstall" => {
            handle_a_warp_uninstall(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        "a_warp_uninstall_confirm" => {
            handle_a_warp_uninstall_confirm(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        _ => {}
    }
    Ok(())
}