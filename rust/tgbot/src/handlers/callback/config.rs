//! 配置删除管理模块

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use std::sync::Arc;
use std::fs;

use crate::app::state::AppState;
use crate::logic::config::{ConfigManager, Proto};
use tgbot::core::error::{Result, AppError};

/// HTML 转义
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn validate_idx(idx: usize, max: usize, field_name: &str) -> Result<()> {
    if idx >= max {
        return Err(AppError::InvalidParameter(format!(
            "{} 索引 {} 超出范围 (最大 {})",
            field_name,
            idx,
            max.saturating_sub(1)
        )));
    }
    Ok(())
}

/// 配置筛选菜单
pub async fn handle_cfg_filter(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let filter = data.strip_prefix("cfg_filter:").unwrap_or("all");
    let filter_label = match filter {
        "reality" => "🌐 Reality",
        "xhttp" => "⚡ XHTTP",
        "kcp" => "📡 KCP",
        _ => "📋 全部",
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📋 全部", "cfg_filter:all"),
            InlineKeyboardButton::callback("🌐 Reality", "cfg_filter:reality"),
            InlineKeyboardButton::callback("⚡ XHTTP", "cfg_filter:xhttp"),
            InlineKeyboardButton::callback("📡 KCP", "cfg_filter:kcp"),
        ],
        vec![InlineKeyboardButton::callback(
            "🧨 删除全部配置",
            format!("cfg_del_all_confirm:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(
            "➗ 按数量删除配置",
            format!("cfg_del_count:{}", filter),
        )],
        vec![InlineKeyboardButton::callback(
            "🎯 指定配置删除",
            format!("cfg_del_select:{}", filter),
        )],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🗑️ <b>删除管理</b> — 当前筛选：{}\n\n请选择删除方式 (操作不可逆):", filter_label),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 删除管理主菜单
pub async fn handle_m_del_cfg(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📋 全部", "cfg_filter:all"),
            InlineKeyboardButton::callback("🌐 Reality", "cfg_filter:reality"),
            InlineKeyboardButton::callback("⚡ XHTTP", "cfg_filter:xhttp"),
            InlineKeyboardButton::callback("📡 KCP", "cfg_filter:kcp"),
        ],
        vec![InlineKeyboardButton::callback(
            "🧨 删除全部配置",
            "cfg_del_all_confirm:all",
        )],
        vec![InlineKeyboardButton::callback(
            "➗ 按数量删除配置",
            "cfg_del_count:all",
        )],
        vec![InlineKeyboardButton::callback(
            "🎯 指定配置删除",
            "cfg_del_select:all",
        )],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🗑️ <b>删除管理</b> — 当前筛选：📋 全部\n\n请选择删除方式 (操作不可逆):",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// ML-DSA-65 管理菜单
pub async fn handle_m_pq_mgmt(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let configured = ConfigManager::is_reality_pq_configured();
    let status = if configured {
        "🟢 已启用（新生成的 Reality 链接将包含 pqv/mldsa65Verify）"
    } else {
        "🔴 未配置（Reality 链接不含 PQ 后量子签名）"
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🗑 删除并禁用", "m_pq_del")],
        vec![InlineKeyboardButton::callback(
            "🔄 初始化 (生成新密钥对)",
            "m_pq_init",
        )],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🔐 <b>ML-DSA-65 管理</b>\n\n当前状态: {}\n\n• <b>删除并禁用</b>: 删除 seed/verify 文件，之后新链接不再带 pqv。\n• <b>初始化</b>: 执行 <code>wwps-core mldsa65</code>（或 xray mldsa65）生成 seed/verify 并写入 /etc/wwps/，与 Xray 完全兼容。\n\n⚠️ 删除或初始化后需<b>重启 Bot</b> 或<b>重新生成批量配置</b>后生效。",
            status
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 删除 ML-DSA-65 密钥
pub async fn handle_m_pq_del(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    match ConfigManager::delete_reality_pq().await {
        Ok(()) => {
            bot.answer_callback_query(q.id.clone())
                .text("✅ 已删除 ML-DSA-65 密钥文件，PQ 已禁用。请重启 Bot 或重新生成配置后生效。")
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
    Ok(())
}

/// 初始化 ML-DSA-65 密钥
pub async fn handle_m_pq_init(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    match ConfigManager::generate_reality_pq_keys().await {
        Ok(()) => {
            bot.answer_callback_query(q.id.clone())
                .text("✅ ML-DSA-65 seed/verify 已通过 wwps-core mldsa65 生成并写入 /etc/wwps/。请重启 Bot 或重新生成配置后生效。")
                .show_alert(true)
                .await?;
        }
        Err(e) => {
            bot.answer_callback_query(q.id.clone())
                .text(format!("❌ 初始化失败: {}", e))
                .show_alert(true)
                .await?;
        }
    }
    Ok(())
}

/// 删除全部配置确认
pub async fn handle_cfg_del_all_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let filter = data.strip_prefix("cfg_del_all_confirm:").unwrap_or("all");
    let filter_type_label = match filter {
        "reality" => "Reality (batch_reality)",
        "xhttp" => "XHTTP (batch_xhttp)",
        "kcp" => "KCP (batch_kcp)",
        _ => "所有",
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "⚠️ 确认清空所有配置 (不可恢复) ⚠️",
            format!("cfg_del_all_exec:{}", filter),
        )],
        vec![InlineKeyboardButton::callback("⬅️ 取消", "m_del_cfg")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🚨 <b>二次确认</b>\n您确定要删除 <b>{}</b> 类型的所有配置文件吗？\n此操作将清空相关 batch_* 文件并重启核心。", filter_type_label),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行删除全部配置
pub async fn handle_cfg_del_all_exec(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let filter = data.strip_prefix("cfg_del_all_exec:").unwrap_or("all");
    let count = if filter == "all" {
        ConfigManager::delete_all_configurations().await.unwrap_or(0)
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => {
                bot.answer_callback_query(q.id.clone())
                    .text("❌ 未知筛选类型")
                    .await?;
                return Ok(());
            }
        };
        let files = ConfigManager::list_inbound_files_by_proto(proto).await.unwrap_or_default();
        let count = files.len();
        for f in &files {
            let _ = fs::remove_file(f);
        }
        if count > 0 {
            let _ = crate::logic::maintenance::MaintenanceManager::reload_core().await;
        }
        count
    };
    bot.answer_callback_query(q.id.clone())
        .text(format!("✅ 已彻底清空 {} 个配置文件", count))
        .show_alert(true)
        .await?;
    Ok(())
}

/// 按数量删除配置
pub async fn handle_cfg_del_count(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let filter = data.strip_prefix("cfg_del_count:").unwrap_or("all");
    let filter_label = match filter {
        "reality" => "🌐 Reality",
        "xhttp" => "⚡ XHTTP",
        "kcp" => "📡 KCP",
        _ => "📋 全部",
    };
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("10 个", format!("cfg_del_exec_count:{}:10", filter)),
            InlineKeyboardButton::callback("50 个", format!("cfg_del_exec_count:{}:50", filter)),
        ],
        vec![
            InlineKeyboardButton::callback("100 个", format!("cfg_del_exec_count:{}:100", filter)),
            InlineKeyboardButton::callback("500 个", format!("cfg_del_exec_count:{}:500", filter)),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "cfg_filter:all")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("➗ <b>按数量删除 ({})</b>\n请选择要删除的文件数量:", filter_label),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行按数量删除
pub async fn handle_cfg_del_exec_count(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let n: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files().await.unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto).await.unwrap_or_default()
    };

    let mut file_with_time = Vec::new();
    for f in files {
        if let Ok(meta) = std::fs::metadata(&f) {
            if let Ok(time) = meta.modified() {
                file_with_time.push((f, time));
            }
        }
    }
    file_with_time.sort_by(|a, b| a.1.cmp(&b.1));

    let to_delete = file_with_time.iter().take(n);
    let mut deleted_count = 0;
    for (f, _) in to_delete {
        if fs::remove_file(f).is_ok() {
            deleted_count += 1;
        }
    }
    if deleted_count > 0 {
        let _ = crate::logic::maintenance::MaintenanceManager::reload_core().await;
    }
    bot.answer_callback_query(q.id.clone())
        .text(format!("✅ 已成功清理 {} 个旧配置", deleted_count))
        .show_alert(true)
        .await?;
    Ok(())
}

/// 指定配置删除
pub async fn handle_cfg_del_select(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let filter = data.strip_prefix("cfg_del_select:").unwrap_or("all");
    let files = if filter == "all" {
        ConfigManager::list_all_inbound_files().await.unwrap_or_default()
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto).await.unwrap_or_default()
    };
    let filter_label = match filter {
        "reality" => "🌐 Reality",
        "xhttp" => "⚡ XHTTP",
        "kcp" => "📡 KCP",
        _ => "📋 全部",
    };
    let mut buttons = Vec::new();
    for (i, path) in files.iter().enumerate().take(50) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        buttons.push(vec![InlineKeyboardButton::callback(
            format!("🗑 {}", filename),
            format!("cfg_del_file:{}:{}", filter, i),
        )]);
    }
    buttons.push(vec![InlineKeyboardButton::callback("🔙 返回筛选", "cfg_filter:all")]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🎯 <b>指定配置删除 ({})</b>\n点击以永久删除对应文件:", filter_label),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// 执行指定配置删除
pub async fn handle_cfg_del_file(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files().await.unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto).await.unwrap_or_default()
    };

    if let Some(path) = files.get(idx) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        let keyboard = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "⚠️ 确认删除",
                format!("cfg_del_confirm:{}:{}", filter, idx),
            )],
            vec![InlineKeyboardButton::callback("🔙 取消", format!("cfg_del_select:{}", filter))],
        ]);
        bot.edit_message_text(
            chat_id,
            msg_id,
            format!("⚠️ <b>删除确认</b>\n\n您确定要删除配置文件 <code>{}</code> 吗？\n此操作不可恢复！", escape_html(filename)),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    } else {
        bot.answer_callback_query(q.id)
            .text("❌ 文件不存在或已被删除")
            .await?;
    }
    Ok(())
}

/// 删除配置确认
pub async fn handle_cfg_del_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files().await.unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto).await.unwrap_or_default()
    };

    if let Err(e) = validate_idx(idx, files.len(), "配置文件") {
        bot.answer_callback_query(q.id.clone())
            .text(&format!("❌ {}", e))
            .await?;
        return Ok(());
    }

    if let Some(path) = files.get(idx) {
        let _ = ConfigManager::delete_specific_configuration(path).await;
        bot.answer_callback_query(q.id.clone())
            .text("✅ 文件已永久删除")
            .show_alert(true)
            .await?;
    } else {
        bot.answer_callback_query(q.id.clone())
            .text("❌ 文件不存在")
            .show_alert(true)
            .await?;
    }
    Ok(())
}

/// 配置删除回调分派
pub async fn dispatch_callback(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: &Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        d if d.starts_with("cfg_filter:") => {
            handle_cfg_filter(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "m_del_cfg" => handle_m_del_cfg(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "m_pq_mgmt" => handle_m_pq_mgmt(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "m_pq_del" => handle_m_pq_del(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "m_pq_init" => handle_m_pq_init(bot.clone(), q.clone(), chat_id, msg_id).await?,
        d if d.starts_with("cfg_del_all_confirm:") => {
            handle_cfg_del_all_confirm(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("cfg_del_all_exec:") => {
            handle_cfg_del_all_exec(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "cfg_del_all_exec" => {
            handle_cfg_del_all_exec(bot.clone(), q.clone(), chat_id, msg_id, "cfg_del_all_exec:all").await?
        }
        d if d.starts_with("cfg_del_count:") => {
            handle_cfg_del_count(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "cfg_del_count" => {
            handle_cfg_del_count(bot.clone(), q.clone(), chat_id, msg_id, "cfg_del_count:all").await?
        }
        d if d.starts_with("cfg_del_exec_count:") => {
            handle_cfg_del_exec_count(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "cfg_del_select" | "cfg_del_select:all" => {
            handle_cfg_del_select(bot.clone(), q.clone(), chat_id, msg_id, data).await?
        }
        d if d.starts_with("cfg_del_file:") => {
            handle_cfg_del_file(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("cfg_del_confirm:") => {
            handle_cfg_del_confirm(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        _ => {}
    }
    Ok(())
}