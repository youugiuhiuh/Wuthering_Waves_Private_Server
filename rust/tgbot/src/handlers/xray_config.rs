use std::fs;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};

use crate::app::state::AppState;
use crate::handlers::CallbackOutcome;
use crate::handlers::utils::{escape_html, validate_idx};
use tgbot::logic::config::{ConfigManager, Proto};
use tgbot::logic::maintenance::MaintenanceManager;

pub async fn handle_xray_config_callback(
    bot: Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    q_id: &str,
    _state: Arc<AppState>,
) -> Result<CallbackOutcome, anyhow::Error> {
    match data {
        "m_xray_mgmt" => {
            let inbounds = ConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let mut buttons = Vec::new();

            if inbounds.is_empty() {
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Reality 批量备份", "u_batch_init"),
                    InlineKeyboardButton::callback("🚀 Xhttp 批量备份", "u_xhttp_batch_init"),
                ]);
                buttons.push(vec![
                    InlineKeyboardButton::callback("🔐 ML-DSA-65 管理", "m_pq_mgmt"),
                ]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "🅧 <b>Xray-core 管理</b>\n\n⚠️ <b>未找到用户配置文件</b>\n\n检测到 Xray-core 已安装，但没有找到用户配置文件(*_inbounds.json)。\n\n您可以：\n• 创建 Reality 批量备份\n• 创建 Xhttp 批量备份\n• 管理 ML-DSA-65 (Reality PQ)\n• 检查配置文件是否正确放置",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            } else {
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("📁 {}", filename),
                        format!("u_l:{}", i),
                    )]);
                }
                buttons.push(vec![InlineKeyboardButton::callback("🗑️ 删除管理", "m_del_cfg")]);
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Reality 批量备份", "u_batch_init"),
                    InlineKeyboardButton::callback("🚀 Xhttp 批量备份", "u_xhttp_batch_init"),
                ]);
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 KCP (mKCP+FinalMask)", "u_kcp_init"),
                    InlineKeyboardButton::callback("🔐 ML-DSA-65 管理", "m_pq_mgmt"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "🅧 <b>Xray-core 管理</b>\n选择配置文件 (支持批量删除):",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_l:") => {
            let idx: usize = data.strip_prefix("u_l:").unwrap_or("0").parse().unwrap_or(0);
            let inbounds = ConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            if let Err(e) = validate_idx(idx, inbounds.len(), "用户配置") {
                bot.answer_callback_query(q_id)
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(CallbackOutcome::Done);
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
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    format!(
                        "👥 <b>用户列表</b>\n文件: <code>{}</code>",
                        path.split('/').next_back().unwrap_or("Unknown")
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_d:") => {
            let parts: Vec<&str> = data.strip_prefix("u_d:").unwrap_or(data).split(':').collect();
            if parts.len() == 2 {
                let idx: usize = parts[0].parse().unwrap_or(0);
                let email = parts[1];
                let inbounds = ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default();
                if let Some(_path) = inbounds.get(idx) {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "⚠️ 确认删除",
                            format!("u_d_confirm:{}:{}", idx, email),
                        )],
                        vec![InlineKeyboardButton::callback("🔙 取消", format!("u_l:{}", idx))],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!(
                            "⚠️ <b>删除确认</b>\n\n您确定要删除用户 <code>{}</code> 吗？\n(注意：当前版本暂未实现单个用户删除逻辑，此操作可能仅用于演示 UI)",
                            escape_html(email)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                } else {
                    bot.answer_callback_query(q_id)
                        .text("❌ 配置文件不存在")
                        .await?;
                }
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_d_confirm:") => {
            let parts: Vec<&str> = data.strip_prefix("u_d_confirm:").unwrap_or(data).split(':').collect();
            if parts.len() == 2 {
                let email = parts[1];
                bot.answer_callback_query(q_id)
                    .text(format!("🗑 暂不支持删除单个用户: {}", email))
                    .show_alert(true)
                    .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("cfg_filter:") => {
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
                format!(
                    "🗑️ <b>删除管理</b> — 当前筛选：{}\n\n请选择删除方式 (操作不可逆):",
                    filter_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(CallbackOutcome::Done)
        }
        "m_del_cfg" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("📋 全部", "cfg_filter:all"),
                    InlineKeyboardButton::callback("🌐 Reality", "cfg_filter:reality"),
                    InlineKeyboardButton::callback("⚡ XHTTP", "cfg_filter:xhttp"),
                    InlineKeyboardButton::callback("📡 KCP", "cfg_filter:kcp"),
                ],
                vec![InlineKeyboardButton::callback("🧨 删除全部配置", "cfg_del_all_confirm:all")],
                vec![InlineKeyboardButton::callback("➗ 按数量删除配置", "cfg_del_count:all")],
                vec![InlineKeyboardButton::callback("🎯 指定配置删除", "cfg_del_select:all")],
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
            Ok(CallbackOutcome::Done)
        }
        "m_pq_mgmt" => {
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
            Ok(CallbackOutcome::Done)
        }
        "m_pq_del" => {
            match ConfigManager::delete_reality_pq().await {
                Ok(()) => {
                    bot.answer_callback_query(q_id)
                        .text("✅ 已删除 ML-DSA-65 密钥文件，PQ 已禁用。请重启 Bot 或重新生成配置后生效。")
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    bot.answer_callback_query(q_id)
                        .text(format!("❌ 删除失败: {}", e))
                        .show_alert(true)
                        .await?;
                }
            }
            Ok(CallbackOutcome::Redirect("m_pq_mgmt".to_string()))
        }
        "m_pq_init" => {
            match ConfigManager::generate_reality_pq_keys().await {
                Ok(()) => {
                    bot.answer_callback_query(q_id)
                        .text("✅ ML-DSA-65 seed/verify 已通过 wwps-core mldsa65 生成并写入 /etc/wwps/。请重启 Bot 或重新生成配置后生效。")
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    bot.answer_callback_query(q_id)
                        .text(format!("❌ 初始化失败: {}", e))
                        .show_alert(true)
                        .await?;
                }
            }
            Ok(CallbackOutcome::Redirect("m_pq_mgmt".to_string()))
        }
        d if d == "cfg_del_all_confirm" || d.starts_with("cfg_del_all_confirm:") => {
            let filter = d.strip_prefix("cfg_del_all_confirm:").unwrap_or("all");
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
                format!(
                    "🚨 <b>二次确认</b>\n您确定要删除 <b>{}</b> 类型的所有配置文件吗？\n此操作将清空相关 batch_* 文件并重启核心。",
                    filter_type_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(CallbackOutcome::Done)
        }
        d if d == "cfg_del_all_exec" || d.starts_with("cfg_del_all_exec:") => {
            let filter = d.strip_prefix("cfg_del_all_exec:").unwrap_or("all");
            let count = if filter == "all" {
                ConfigManager::delete_all_configurations()
                    .await
                    .unwrap_or(0)
            } else {
                let proto = match filter {
                    "reality" => Proto::Vision,
                    "xhttp" => Proto::XHTTP,
                    "kcp" => Proto::Kcp,
                    _ => {
                        bot.answer_callback_query(q_id)
                            .text("❌ 未知筛选类型")
                            .await?;
                        return Ok(CallbackOutcome::Redirect("m_del_cfg".to_string()));
                    }
                };
                let files = ConfigManager::list_inbound_files_by_proto(proto)
                    .await
                    .unwrap_or_default();
                let count = files.len();
                for f in &files {
                    let _ = fs::remove_file(f);
                }
                if count > 0 {
                    let _ = MaintenanceManager::reload_core().await;
                }
                count
            };
            bot.answer_callback_query(q_id)
                .text(format!("✅ 已彻底清空 {} 个配置文件", count))
                .show_alert(true)
                .await?;
            Ok(CallbackOutcome::Redirect("m_del_cfg".to_string()))
        }
        d if d == "cfg_del_count" || d.starts_with("cfg_del_count:") => {
            let filter = d.strip_prefix("cfg_del_count:").unwrap_or("all");
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
                format!(
                    "➗ <b>按数量删除 ({})</b>\n请选择要删除的文件数量:",
                    filter_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("cfg_del_exec_count:") => {
            let parts: Vec<&str> = data.split(':').collect();
            let filter = parts.get(1).unwrap_or(&"all");
            let n: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

            let files = if *filter == "all" {
                ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default()
            } else {
                let proto = match *filter {
                    "reality" => Proto::Vision,
                    "xhttp" => Proto::XHTTP,
                    "kcp" => Proto::Kcp,
                    _ => Proto::Vision,
                };
                ConfigManager::list_inbound_files_by_proto(proto)
                    .await
                    .unwrap_or_default()
            };

            let mut file_with_time = Vec::new();
            for f in files {
                if let Ok(meta) = std::fs::metadata(&f)
                    && let Ok(time) = meta.modified()
                {
                    file_with_time.push((f, time));
                }
            }
            file_with_time.sort_by_key(|a| a.1);

            let to_delete = file_with_time.iter().take(n);
            let mut deleted_count = 0;
            for (f, _) in to_delete {
                if fs::remove_file(f).is_ok() {
                    deleted_count += 1;
                }
            }
            if deleted_count > 0 {
                let _ = MaintenanceManager::reload_core().await;
            }
            bot.answer_callback_query(q_id)
                .text(format!("✅ 已成功清理 {} 个旧配置", deleted_count))
                .show_alert(true)
                .await?;
            Ok(CallbackOutcome::Redirect(format!("cfg_del_count:{}", filter)))
        }
        d if d == "cfg_del_select" || d.starts_with("cfg_del_select:") => {
            let filter = d.strip_prefix("cfg_del_select:").unwrap_or("all");
            let files = if filter == "all" {
                ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default()
            } else {
                let proto = match filter {
                    "reality" => Proto::Vision,
                    "xhttp" => Proto::XHTTP,
                    "kcp" => Proto::Kcp,
                    _ => Proto::Vision,
                };
                ConfigManager::list_inbound_files_by_proto(proto)
                    .await
                    .unwrap_or_default()
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
            buttons.push(vec![InlineKeyboardButton::callback(
                "🔙 返回筛选",
                "cfg_filter:all",
            )]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🎯 <b>指定配置删除 ({})</b>\n点击以永久删除对应文件:",
                    filter_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("cfg_del_file:") => {
            let parts: Vec<&str> = data.split(':').collect();
            let filter = parts.get(1).unwrap_or(&"all");
            let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

            let files = if *filter == "all" {
                ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default()
            } else {
                let proto = match *filter {
                    "reality" => Proto::Vision,
                    "xhttp" => Proto::XHTTP,
                    "kcp" => Proto::Kcp,
                    _ => Proto::Vision,
                };
                ConfigManager::list_inbound_files_by_proto(proto)
                    .await
                    .unwrap_or_default()
            };

            if let Some(path) = files.get(idx) {
                let filename = path.split('/').next_back().unwrap_or("Unknown");
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "⚠️ 确认删除",
                        format!("cfg_del_confirm:{}:{}", filter, idx),
                    )],
                    vec![InlineKeyboardButton::callback(
                        "🔙 取消",
                        format!("cfg_del_select:{}", filter),
                    )],
                ]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    format!(
                        "⚠️ <b>删除确认</b>\n\n您确定要删除配置文件 <code>{}</code> 吗？\n此操作不可恢复！",
                        escape_html(filename)
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            } else {
                bot.answer_callback_query(q_id)
                    .text("❌ 文件不存在或已被删除")
                    .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("cfg_del_confirm:") => {
            let parts: Vec<&str> = data.split(':').collect();
            let filter = parts.get(1).unwrap_or(&"all");
            let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

            let files = if *filter == "all" {
                ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default()
            } else {
                let proto = match *filter {
                    "reality" => Proto::Vision,
                    "xhttp" => Proto::XHTTP,
                    "kcp" => Proto::Kcp,
                    _ => Proto::Vision,
                };
                ConfigManager::list_inbound_files_by_proto(proto)
                    .await
                    .unwrap_or_default()
            };

            if let Err(e) = validate_idx(idx, files.len(), "配置文件") {
                bot.answer_callback_query(q_id)
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(CallbackOutcome::Done);
            }

            if let Some(path) = files.get(idx) {
                let _ = ConfigManager::delete_specific_configuration(path).await;
                bot.answer_callback_query(q_id)
                    .text("✅ 文件已永久删除")
                    .show_alert(true)
                    .await?;
            } else {
                bot.answer_callback_query(q_id)
                    .text("❌ 文件不存在")
                    .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ => Ok(CallbackOutcome::Done),
    }
}
