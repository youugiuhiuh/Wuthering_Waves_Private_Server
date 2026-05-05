//! Xray 核心配置回调处理模块

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode};
use std::sync::Arc;
use std::time::Duration;

use crate::app::state::AppState;
use crate::logic::config::{ConfigManager, KcpMask, Proto};
use crate::logic::maintenance::MaintenanceManager;
use crate::logic::system::SystemMonitor;
use tgbot::core::types::IpVersion;
use tgbot::core::error::{Result, AppError};
use crate::handlers::proxy::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};

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

/// Xray 核心配置回调处理
///
/// 支持 Reality 批量创建、KCP 遮罩配置、用户管理等功能.
///
/// # Arguments
/// * `bot` - Telegram bot 实例
/// * `q` - 回调查询
/// * `chat_id` - 聊天 ID
/// * `msg_id` - 消息 ID
/// * `data` - callback data
/// * `state` - 应用状态
///
/// # Returns
/// 处理结果
pub async fn handle_xraycore_callback(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        "u_batch_init" => {
            if MaintenanceManager::is_reality_base_ready().await {
                show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision)
                    .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("⏳ 正在准备 Reality 母版，请稍候...")
                    .await?;
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "⏳ <b>正在自动初始化 Reality 基础环境...</b>\n请稍候，完成后会自动进入批量生产界面。",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
            }
        }
        "u_xhttp_batch_init" => {
            if MaintenanceManager::is_reality_base_ready().await {
                show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::XHTTP)
                    .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("⏳ 正在准备 Reality 母版，请稍候...")
                    .await?;
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "⏳ <b>正在自动初始化 Reality 基础环境...</b>\n请稍候，完成后会自动进入批量生产界面。",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
            }
        }
        d if d.starts_with("u_batch_ip_init:") || d.starts_with("u_xhttp_batch_ip_init:") => {
            let (prefix, proto) = if d.starts_with("u_batch_ip_init:") {
                ("u_batch_ip_init:", Proto::Vision)
            } else {
                ("u_xhttp_batch_ip_init:", Proto::XHTTP)
            };
            let ip_ver_code = d.strip_prefix(prefix).unwrap_or("");
            let ip_version = match ip_ver_code {
                "6" => IpVersion::IPv6,
                "s6" => IpVersion::SplitStackV6Primary,
                "s4" => IpVersion::SplitStackV4Primary,
                _ => IpVersion::IPv4,
            };
            show_reality_qty_prompt(&bot, chat_id, msg_id, ip_version, proto).await?;
        }
        d if d.starts_with("u_batch_exec:") || d.starts_with("u_xhttp_batch_exec:") => {
            let (prefix, proto) = if d.starts_with("u_batch_exec:") {
                ("u_batch_exec:", Proto::Vision)
            } else {
                ("u_xhttp_batch_exec:", Proto::XHTTP)
            };
            let parts: Vec<&str> = d.strip_prefix(prefix).unwrap_or(d).split(':').collect();
            if parts.len() != 2 {
                return Ok(());
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
                bot.answer_callback_query(q.id.clone())
                    .text("⚙️ 基础配置缺失，正在自动初始化...")
                    .await?;
                trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                return Ok(());
            }

            let ip_str = match ip_version {
                IpVersion::IPv4 => "IPv4",
                IpVersion::IPv6 => "IPv6",
                IpVersion::SplitStackV6Primary => "双栈分离 (v6上v4下)",
                IpVersion::SplitStackV4Primary => "双栈分离 (v4上v6下)",
            };

            let proto_str = match proto {
                Proto::Vision => "Reality",
                Proto::XHTTP => "XHTTP",
                Proto::Kcp => "KCP",
            };

            bot.answer_callback_query(q.id.clone())
                .text(format!(
                    "⏳ 正在生成 {} 个 {} 增强配置 ({}, 独立文件)...",
                    n, proto_str, ip_str
                ))
                .await?;

            let res = match proto {
                Proto::Vision => {
                    ConfigManager::batch_create_reality_vision_enhanced(n, ip_version).await
                }
                Proto::XHTTP => {
                    ConfigManager::batch_create_xhttp_reality_enhanced(n, ip_version).await
                }
                Proto::Kcp => {
                    unreachable!("KCP uses separate batch handler")
                }
            };

            match res {
                Ok(result) => {
                    let mut message_ids: Vec<MessageId> = Vec::new();
                    let mut combined_links = String::new();
                    for (i, link) in result.links.iter().enumerate() {
                        combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                        if (i + 1) % 2 == 0 {
                            if let Ok(msg) = bot
                                .send_message(chat_id, combined_links.clone())
                                .parse_mode(ParseMode::Html)
                                .await
                            {
                                message_ids.push(msg.id);
                            }
                            combined_links.clear();
                        }
                    }
                    if !combined_links.is_empty() {
                        if let Ok(msg) = bot
                            .send_message(chat_id, combined_links)
                            .parse_mode(ParseMode::Html)
                            .await
                        {
                            message_ids.push(msg.id);
                        }
                    }

                    let links_text = result.links.join("\n");
                    let timestamp = chrono::Utc::now().timestamp();
                    let temp_file_path = format!("/tmp/wwps_reality_links_{}.txt", timestamp);

                    if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                        log::warn!("写入临时文件失败: {}", e);
                    } else {
                        let document_sent = bot
                            .send_document(chat_id, InputFile::file(&temp_file_path))
                            .caption("完整链接列表，建议尽快复制/导入")
                            .await;

                        if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                            log::warn!("删除临时文件失败: {}", e);
                        }

                        if let Ok(msg) = document_sent {
                            message_ids.push(msg.id);
                        }
                    }

                    let mut result_msg = format!(
                        "✅ 增强批量生成完成！\n\n📊 生成数量: {}\n🌐 网络协议: {}\n🔒 安全特性: 随机ShortId、去重SNI、唯一Tag",
                        result.created_count, ip_str
                    );

                    if let Some(filename) = result.config_file {
                        result_msg.push_str(&format!("\n\n📁 独立配置文件: {}", filename));
                    }

                    if let Some(backup_file) = result.backup_file {
                        result_msg.push_str(&format!("\n💾 原配置备份: {}", backup_file));
                    }

                    let summary_msg = bot.send_message(chat_id, result_msg).await?;
                    message_ids.push(summary_msg.id);

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        for msg_id in message_ids {
                            if let Err(e) = bot_clone.delete_message(chat_id_clone, msg_id).await {
                                log::warn!("删除消息失败 (chat_id: {}, msg_id: {}): {}", chat_id_clone, msg_id, e);
                            }
                        }
                    });
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("未找到 Reality 配置文件") {
                        bot.send_message(chat_id, "⚠️ <b>检测到 Reality 母版缺失，正在自动初始化...</b>")
                            .parse_mode(ParseMode::Html)
                            .await?;
                        trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                    } else {
                        bot.send_message(chat_id, format!("❌ 生成失败: {}", err_msg)).await?;
                    }
                }
            }
        }
        "u_kcp_init" => {
            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
            buttons.push(vec![
                InlineKeyboardButton::callback("🔐 加密层 (2)", "u_kcp_cat:enc"),
                InlineKeyboardButton::callback("🌀 混淆层 (3)", "u_kcp_cat:obf"),
            ]);
            buttons.push(vec![
                InlineKeyboardButton::callback("🎭 伪装层 (6)", "u_kcp_cat:dis"),
                InlineKeyboardButton::callback("⚡ 扩展层 (3)", "u_kcp_cat:ext"),
            ]);
            buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")]);

            bot.edit_message_text(
                chat_id,
                msg_id,
                "🚀 <b>KCP (mKCP+FinalMask) 配置</b>\n\n\
                 ✨ <b>特点:</b>\n\
                 • 基于 mKCP 协议的可靠传输\n\
                 • FinalMask 多层遮罩任意叠加(1-5层)\n\
                 • 支持加密、混淆、伪装、扩展四大类遮罩\n\n\
                 📋 <b>步骤 1: 选择遮罩类别</b>\n\
                 ⚠️ 至少选择1层，建议加密层+伪装层组合",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_cat:") => {
            let cat_code = d.strip_prefix("u_kcp_cat:").unwrap_or("enc");
            let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("未知");
            let variants = KcpMask::variants_by_category(cat_code);
            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

            for mask in &variants {
                buttons.push(vec![InlineKeyboardButton::callback(
                    format!("✅ {}", mask.display_name()),
                    format!("u_kcp_add:{}", mask.code()),
                )]);
            }

            buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回分类", "u_kcp_init")]);

            let mask_list: String = variants
                .iter()
                .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
                .collect::<Vec<_>>()
                .join("\n\n");

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!("<b>{}</b> — 选择要添加的遮罩\n\n{}", cat_name, mask_list),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_add:") => {
            let code = d.strip_prefix("u_kcp_add:").unwrap_or("mo");
            if let Some(m) = KcpMask::from_code(code) {
                if let Err(e) = m.is_compatible_with(&[]) {
                    bot.answer_callback_query(q.id.clone())
                        .text(format!("❌ {}", e))
                        .await?;
                    return Ok(());
                }
                let stack_display = format!("1️⃣ {}", m.display_name());
                let buttons = vec![
                    vec![InlineKeyboardButton::callback("➕ 继续添加遮罩层", format!("u_kcp_more:{}", code))],
                    vec![InlineKeyboardButton::callback("✅ 完成配置", format!("u_kcp_done:{}", code))],
                    vec![InlineKeyboardButton::callback("🗑️ 清空重选", "u_kcp_init")],
                ];
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    format!("📋 <b>当前遮罩栈:</b>\n{}\n\n➕ 可以继续添加，或完成配置", stack_display),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            }
        }
        d if d.starts_with("u_kcp_more:") => {
            let existing = d.strip_prefix("u_kcp_more:").unwrap_or("");
            let existing_codes: Vec<&str> = existing.split(',').collect();

            let stack_display: Vec<String> = existing_codes.iter().enumerate().map(|(i, c)| {
                let m = KcpMask::from_code(c);
                format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
            }).collect();

            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

            let current_masks: Vec<KcpMask> = existing_codes
                .iter()
                .filter(|c| !c.is_empty())
                .filter_map(|c| KcpMask::from_code(c))
                .collect();

            let has_sudoku = current_masks.iter().any(|m| m.is_sudoku());
            let has_encryption = current_masks.iter().any(|m| m.is_encryption());

            let cat_counts = [
                ("enc", "🔐 加密层", KcpMask::variants_by_category("enc").len()),
                ("obf", "🌀 混淆层", KcpMask::variants_by_category("obf").len()),
                ("dis", "🎭 伪装层", KcpMask::variants_by_category("dis").len()),
                ("ext", "⚡ 扩展层", KcpMask::variants_by_category("ext").len()),
            ];

            for (code, name, total) in &cat_counts {
                let added_count = existing_codes.iter().filter(|ec| {
                    KcpMask::from_code(ec).map(|m| m.category_code() == *code).unwrap_or(false)
                }).count();
                let remaining = total - added_count;

                let disabled_reason = match *code {
                    "enc" if has_encryption => Some("已添加"),
                    "obf" if has_sudoku => Some("数独已添加"),
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
                    buttons.last_mut().unwrap().push(
                        InlineKeyboardButton::callback(
                            format!("{} ({})", name, remaining),
                            format!("u_kcp_mcat:{}:{}", existing, code),
                        ),
                    );
                }
            }

            buttons.push(vec![InlineKeyboardButton::callback("✅ 完成配置", format!("u_kcp_done:{}", existing))]);
            buttons.push(vec![InlineKeyboardButton::callback("🗑️ 清空重选", "u_kcp_init")]);

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n➕ <b>选择要添加的遮罩类别</b> (已达{}层)",
                    stack_display.join("\n"),
                    existing_codes.len()
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_mcat:") => {
            let data = d.strip_prefix("u_kcp_mcat:").unwrap_or("");
            let parts: Vec<&str> = data.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(());
            }
            let existing = parts[0];
            let cat_code = parts[1];
            let existing_codes: Vec<&str> = existing.split(',').collect();
            let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("未知");

            let variants = KcpMask::variants_by_category(cat_code);

            let current_masks: Vec<KcpMask> = existing_codes
                .iter()
                .filter(|c| !c.is_empty())
                .filter_map(|c| KcpMask::from_code(c))
                .collect();

            let stack_display: Vec<String> = existing_codes.iter().enumerate().map(|(i, c)| {
                let m = KcpMask::from_code(c);
                format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
            }).collect();

            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

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

            buttons.push(vec![
                InlineKeyboardButton::callback("⬅️ 返回分类", format!("u_kcp_more:{}", existing)),
            ]);
            buttons.push(vec![InlineKeyboardButton::callback("✅ 完成配置", format!("u_kcp_done:{}", existing))]);
            buttons.push(vec![InlineKeyboardButton::callback("🗑️ 清空重选", "u_kcp_init")]);

            let mask_list: String = variants
                .iter()
                .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
                .collect::<Vec<_>>()
                .join("\n\n");

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n<b>{}</b> — 选择要添加的遮罩\n\n{}",
                    stack_display.join("\n"),
                    cat_name,
                    mask_list
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_push:") => {
            let data = d.strip_prefix("u_kcp_push:").unwrap_or("");
            let parts: Vec<&str> = data.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(());
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
                    bot.answer_callback_query(q.id.clone())
                        .text("❌ 未知遮罩类型")
                        .await?;
                    return Ok(());
                }
            };

            if let Err(e) = new_mask.is_compatible_with(&current_masks) {
                bot.answer_callback_query(q.id.clone())
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(());
            }

            let new_stack = if existing.is_empty() {
                new_code.to_string()
            } else {
                format!("{},{}", existing, new_code)
            };
            let codes: Vec<&str> = new_stack.split(',').collect();

            let stack_display: Vec<String> = codes.iter().enumerate().map(|(i, c)| {
                let m = KcpMask::from_code(c);
                format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
            }).collect();

            let mut buttons = Vec::new();
            buttons.push(vec![InlineKeyboardButton::callback("➕ 继续添加遮罩层", format!("u_kcp_more:{}", new_stack))]);
            buttons.push(vec![InlineKeyboardButton::callback("✅ 完成配置", format!("u_kcp_done:{}", new_stack))]);
            buttons.push(vec![InlineKeyboardButton::callback("🗑️ 清空重选", "u_kcp_init")]);

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n➕ 可以继续添加，或完成配置",
                    stack_display.join("\n"),
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_done:") => {
            let mask_codes_str = d.strip_prefix("u_kcp_done:").unwrap_or("");
            let codes: Vec<&str> = mask_codes_str.split(',').collect();

            if codes.is_empty() {
                bot.answer_callback_query(q.id.clone())
                    .text("❌ 请至少选择1层遮罩")
                    .await?;
                return Ok(());
            }

            let masks: Vec<KcpMask> = codes
                .iter()
                .filter_map(|c| KcpMask::from_code(c))
                .collect();

            let ordered = KcpMask::canonical_order(&masks);

            if let Err(e) = KcpMask::validate_stack(&ordered) {
                bot.answer_callback_query(q.id.clone())
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(());
            }

            let warnings = KcpMask::get_stack_warnings(&ordered);
            let stack_display: Vec<String> = ordered.iter().map(|m| m.display_name().to_string()).collect();

            let ordered_codes: Vec<String> = ordered.iter().map(|m| m.code().to_string()).collect();
            let ordered_str = ordered_codes.join(",");

            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            let mut buttons = vec![vec![
                InlineKeyboardButton::callback("🌐 IPv4 (0.0.0.0)", format!("u_kcp_ip:{}:4", ordered_str)),
            ]];
            if has_ipv6 {
                buttons[0].push(InlineKeyboardButton::callback(
                    "🌐 IPv6 (::)",
                    format!("u_kcp_ip:{}:6", ordered_str),
                ));
            }
            buttons.push(vec![
                InlineKeyboardButton::callback("🔄 双栈 IPv4优先", format!("u_kcp_ip:{}:s4", ordered_str)),
            ]);
            buttons.push(vec![
                InlineKeyboardButton::callback("🔄 双栈 IPv6优先", format!("u_kcp_ip:{}:s6", ordered_str)),
            ]);
            buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", format!("u_kcp_more:{}", mask_codes_str))]);

            let warning_text = if warnings.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", warnings.join("\n"))
            };

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🚀 <b>KCP 配置</b>\n\n📋 <b>遮罩栈 (外层→内层):</b>\n{}{}\n\n⬇️ <b>请选择网络协议版本:</b>",
                    stack_display.join(" → "),
                    warning_text
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_ip:") => {
            let data = d.strip_prefix("u_kcp_ip:").unwrap_or("");
            let last_colon = data.rfind(':').unwrap_or(data.len());
            let mask_codes_str = &data[..last_colon];
            let ip_ver_code = &data[last_colon+1..];
            let codes: Vec<&str> = mask_codes_str.split(',').collect();

            let ip_version: IpVersion = match ip_ver_code {
                "6" => IpVersion::IPv6,
                "s4" => IpVersion::SplitStackV4Primary,
                "s6" => IpVersion::SplitStackV6Primary,
                _ => IpVersion::IPv4,
            };
            let ip_display = match ip_version {
                IpVersion::IPv4 => "IPv4",
                IpVersion::IPv6 => "IPv6",
                IpVersion::SplitStackV4Primary => "双栈 IPv4优先",
                IpVersion::SplitStackV6Primary => "双栈 IPv6优先",
            };

            let stack_display: Vec<String> = codes.iter().enumerate().map(|(i, c)| {
                let m = KcpMask::from_code(c);
                format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
            }).collect();

            let buttons = vec![
                vec![
                    InlineKeyboardButton::callback("1", format!("u_kcp_ok:{}:{}:1", mask_codes_str, ip_ver_code)),
                    InlineKeyboardButton::callback("3", format!("u_kcp_ok:{}:{}:3", mask_codes_str, ip_ver_code)),
                    InlineKeyboardButton::callback("5", format!("u_kcp_ok:{}:{}:5", mask_codes_str, ip_ver_code)),
                ],
                vec![
                    InlineKeyboardButton::callback("10", format!("u_kcp_ok:{}:{}:10", mask_codes_str, ip_ver_code)),
                    InlineKeyboardButton::callback("20", format!("u_kcp_ok:{}:{}:20", mask_codes_str, ip_ver_code)),
                    InlineKeyboardButton::callback("50", format!("u_kcp_ok:{}:{}:50", mask_codes_str, ip_ver_code)),
                ],
                vec![InlineKeyboardButton::callback("⬅️ 返回", format!("u_kcp_done:{}", mask_codes_str))],
            ];

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🚀 <b>KCP 配置 - 批量生成</b>\n\n📋 <b>遮罩栈:</b>\n{}\n\n🌐 网络协议: <b>{}</b>\n\n⬇️ <b>请选择生成数量:</b>",
                    stack_display.join("\n"),
                    ip_display
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
        }
        d if d.starts_with("u_kcp_ok:") => {
            let data = d.strip_prefix("u_kcp_ok:").unwrap_or("");
            let parts: Vec<&str> = data.rsplitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(());
            }
            let n: usize = parts[0].parse().unwrap_or(0);
            let remaining = parts[1];
            let last_colon = remaining.rfind(':').unwrap_or(remaining.len());
            let mask_codes_str = &remaining[..last_colon];
            let ip_ver_code = &remaining[last_colon+1..];

            let ip_version: IpVersion = match ip_ver_code {
                "6" => IpVersion::IPv6,
                "s4" => IpVersion::SplitStackV4Primary,
                "s6" => IpVersion::SplitStackV6Primary,
                _ => IpVersion::IPv4,
            };
            let ip_str = match ip_version {
                IpVersion::IPv4 => "IPv4",
                IpVersion::IPv6 => "IPv6",
                IpVersion::SplitStackV4Primary => "双栈 IPv4优先",
                IpVersion::SplitStackV6Primary => "双栈 IPv6优先",
            };

            let mask_codes: Vec<&str> = mask_codes_str.split(',').collect();
            let mask_names: Vec<&str> = mask_codes.iter()
                .filter_map(|c| KcpMask::from_code(c).map(|m| m.display_name()))
                .collect();
            let mask_label = mask_names.join("+");

            bot.answer_callback_query(q.id.clone())
                .text(format!("⏳ 正在生成 {} 个 KCP 配置...", n))
                .await?;

            let res = ConfigManager::batch_create_kcp(n, ip_version, &mask_codes).await;

            match res {
                Ok(result) => {
                    let mut message_ids: Vec<MessageId> = Vec::new();

                    let mut combined_links = String::new();
                    for (i, link) in result.links.iter().enumerate() {
                        combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                        if (i + 1) % 2 == 0 {
                            if let Ok(msg) = bot
                                .send_message(chat_id, combined_links.clone())
                                .parse_mode(ParseMode::Html)
                                .await
                            {
                                message_ids.push(msg.id);
                            }
                            combined_links.clear();
                        }
                    }
                    if !combined_links.is_empty() {
                        if let Ok(msg) = bot
                            .send_message(chat_id, combined_links)
                            .parse_mode(ParseMode::Html)
                            .await
                        {
                            message_ids.push(msg.id);
                        }
                    }

                    let links_text = result.links.join("\n");
                    let timestamp = chrono::Utc::now().timestamp();
                    let temp_file_path = format!("/tmp/wwps_kcp_links_{}.txt", timestamp);

                    if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                        log::warn!("写入临时文件失败: {}", e);
                    } else {
                        let document_sent = bot
                            .send_document(chat_id, InputFile::file(&temp_file_path))
                            .caption(format!("KCP {} 完整链接列表", mask_label))
                            .await;

                        if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                            log::warn!("删除临时文件失败: {}", e);
                        }

                        if let Ok(msg) = document_sent {
                            message_ids.push(msg.id);
                        }
                    }

                    let mut result_msg = format!(
                        "✅ KCP 批量生成完成！\n\n📊 生成数量: {}\n🌐 网络协议: {}\n🎭 遮罩层: {}",
                        result.created_count, ip_str, mask_label
                    );

                    if let Some(filename) = result.config_file {
                        result_msg.push_str(&format!("\n\n📁 配置文件: {}", filename));
                    }

                    let summary_msg = bot.send_message(chat_id, result_msg).await?;
                    message_ids.push(summary_msg.id);

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        for msg_id in message_ids {
                            let _ = bot_clone.delete_message(chat_id_clone, msg_id).await;
                        }
                    });
                }
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ 生成失败: {}", e))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        d if d.starts_with("u_l:") => {
            let idx: usize = d.strip_prefix("u_l:").unwrap_or("0").parse().unwrap_or(0);
            let inbounds = ConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            if let Err(e) = validate_idx(idx, inbounds.len(), "用户配置") {
                bot.answer_callback_query(q.id.clone())
                    .text(&format!("❌ {}", e))
                    .await?;
                return Ok(());
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
        }
        d if d.starts_with("u_d:") => {
            let parts: Vec<&str> = d.strip_prefix("u_d:").unwrap_or(d).split(':').collect();
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
                        vec![InlineKeyboardButton::callback(
                            "🔙 取消",
                            format!("u_l:{}", idx),
                        )],
                    ]);

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!("⚠️ <b>删除确认</b>\n\n您确定要删除用户 <code>{}</code> 吗？\n(注意：当前版本暂未实现单个用户删除逻辑，此操作可能仅用于演示 UI)", escape_html(email)),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                } else {
                    bot.answer_callback_query(q.id)
                        .text("❌ 配置文件不存在")
                        .await?;
                }
            }
        }
        d if d.starts_with("u_d_confirm:") => {
            let parts: Vec<&str> = d
                .strip_prefix("u_d_confirm:")
                .unwrap_or(d)
                .split(':')
                .collect();
            if parts.len() == 2 {
                let email = parts[1];
                bot.answer_callback_query(q.id.clone())
                    .text(format!("🗑 暂不支持删除单个用户: {}", email))
                    .show_alert(true)
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}
