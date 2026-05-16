use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode};
use tokio::time::Duration;

#[allow(dead_code)]
use crate::app::state::AppState;
use crate::handlers::CallbackOutcome;
use tgbot::core::types::IpVersion;
use tgbot::logic::config::{ConfigManager, KcpMask};
use tgbot::logic::system::SystemMonitor;

#[allow(dead_code)]
pub async fn handle_kcp_callback(
    bot: Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    q_id: &str,
    _state: Arc<AppState>,
) -> Result<CallbackOutcome, anyhow::Error> {
    match data {
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
            buttons.push(vec![InlineKeyboardButton::callback(
                "⬅️ 返回",
                "m_xray_mgmt",
            )]);

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
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_cat:") => {
            let cat_code = data.strip_prefix("u_kcp_cat:").unwrap_or("enc");
            let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("未知");

            let variants = KcpMask::variants_by_category(cat_code);
            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

            for mask in &variants {
                buttons.push(vec![InlineKeyboardButton::callback(
                    format!("✅ {}", mask.display_name()),
                    format!("u_kcp_add:{}", mask.code()),
                )]);
            }

            buttons.push(vec![InlineKeyboardButton::callback(
                "⬅️ 返回分类",
                "u_kcp_init",
            )]);

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
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_add:") => {
            let code = data.strip_prefix("u_kcp_add:").unwrap_or("mo");
            if let Some(m) = KcpMask::from_code(code) {
                if let Err(e) = m.is_compatible_with(&[]) {
                    bot.answer_callback_query(q_id)
                        .text(format!("❌ {}", e))
                        .await?;
                    return Ok(CallbackOutcome::Done);
                }
                let stack_display = format!("1️⃣ {}", m.display_name());
                let buttons = vec![
                    vec![InlineKeyboardButton::callback(
                        "➕ 继续添加遮罩层",
                        format!("u_kcp_more:{}", code),
                    )],
                    vec![InlineKeyboardButton::callback(
                        "✅ 完成配置",
                        format!("u_kcp_done:{}", code),
                    )],
                    vec![InlineKeyboardButton::callback("🗑️ 清空重选", "u_kcp_init")],
                ];
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    format!(
                        "📋 <b>当前遮罩栈:</b>\n{}\n\n\
                         ➕ 可以继续添加，或完成配置",
                        stack_display
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(buttons))
                .await?;
            }
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_more:") => {
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
                ("enc", "🔐 加密层", KcpMask::variants_by_category("enc").len()),
                ("obf", "🌀 混淆层", KcpMask::variants_by_category("obf").len()),
                ("dis", "🎭 伪装层", KcpMask::variants_by_category("dis").len()),
                ("ext", "⚡ 扩展层", KcpMask::variants_by_category("ext").len()),
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
                    buttons
                        .last_mut()
                        .unwrap()
                        .push(InlineKeyboardButton::callback(
                            format!("{} ({})", name, remaining),
                            format!("u_kcp_mcat:{}:{}", existing, code),
                        ));
                } else {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("⛔ {} (已达上限)", name),
                        "noop",
                    )]);
                }
            }

            buttons.push(vec![InlineKeyboardButton::callback(
                "✅ 完成配置",
                format!("u_kcp_done:{}", existing),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "🗑️ 清空重选",
                "u_kcp_init",
            )]);

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n\
                     ➕ <b>选择要添加的遮罩类别</b> (已达{}层)",
                    stack_display.join("\n"),
                    existing_codes.len()
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_mcat:") => {
            let data_str = data.strip_prefix("u_kcp_mcat:").unwrap_or("");
            let parts: Vec<&str> = data_str.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(CallbackOutcome::Done);
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

            let stack_display: Vec<String> = existing_codes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let m = KcpMask::from_code(c);
                    format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
                })
                .collect();

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

            buttons.push(vec![InlineKeyboardButton::callback(
                "⬅️ 返回分类",
                format!("u_kcp_more:{}", existing),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "✅ 完成配置",
                format!("u_kcp_done:{}", existing),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "🗑️ 清空重选",
                "u_kcp_init",
            )]);

            let mask_list: String = variants
                .iter()
                .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
                .collect::<Vec<_>>()
                .join("\n\n");

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n\
                     <b>{}</b> — 选择要添加的遮罩\n\n{}",
                    stack_display.join("\n"),
                    cat_name,
                    mask_list
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_push:") => {
            let data_str = data.strip_prefix("u_kcp_push:").unwrap_or("");
            let parts: Vec<&str> = data_str.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(CallbackOutcome::Done);
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
                    bot.answer_callback_query(q_id)
                        .text("❌ 未知遮罩类型")
                        .await?;
                    return Ok(CallbackOutcome::Done);
                }
            };

            if let Err(e) = new_mask.is_compatible_with(&current_masks) {
                bot.answer_callback_query(q_id)
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(CallbackOutcome::Done);
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
                "➕ 继续添加遮罩层",
                format!("u_kcp_more:{}", new_stack),
            )]);

            buttons.push(vec![InlineKeyboardButton::callback(
                "✅ 完成配置",
                format!("u_kcp_done:{}", new_stack),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "🗑️ 清空重选",
                "u_kcp_init",
            )]);

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "📋 <b>当前遮罩栈:</b>\n{}\n\n\
                     ➕ 可以继续添加，或完成配置",
                    stack_display.join("\n"),
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_done:") => {
            let mask_codes_str = data.strip_prefix("u_kcp_done:").unwrap_or("");
            let codes: Vec<&str> = mask_codes_str.split(',').collect();

            if codes.is_empty() {
                bot.answer_callback_query(q_id)
                    .text("❌ 请至少选择1层遮罩")
                    .await?;
                return Ok(CallbackOutcome::Done);
            }

            let masks: Vec<KcpMask> =
                codes.iter().filter_map(|c| KcpMask::from_code(c)).collect();

            let ordered = KcpMask::canonical_order(&masks);

            if let Err(e) = KcpMask::validate_stack(&ordered) {
                bot.answer_callback_query(q_id)
                    .text(format!("❌ {}", e))
                    .await?;
                return Ok(CallbackOutcome::Done);
            }

            let warnings = KcpMask::get_stack_warnings(&ordered);
            let stack_display: Vec<String> = ordered
                .iter()
                .map(|m| m.display_name().to_string())
                .collect();

            let ordered_codes: Vec<String> =
                ordered.iter().map(|m| m.code().to_string()).collect();
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
                "🔄 双栈 IPv4优先",
                format!("u_kcp_ip:{}:s4", ordered_str),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "🔄 双栈 IPv6优先",
                format!("u_kcp_ip:{}:s6", ordered_str),
            )]);
            buttons.push(vec![InlineKeyboardButton::callback(
                "⬅️ 返回",
                format!("u_kcp_more:{}", mask_codes_str),
            )]);

            let warning_text = if warnings.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", warnings.join("\n"))
            };

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🚀 <b>KCP 配置</b>\n\n\
                     📋 <b>遮罩栈 (外层→内层):</b>\n{}{}\n\n\
                     ⬇️ <b>请选择网络协议版本:</b>",
                    stack_display.join(" → "),
                    warning_text
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_ip:") => {
            let data_str = data.strip_prefix("u_kcp_ip:").unwrap_or("");
            let last_colon = data_str.rfind(':').unwrap_or(data_str.len());
            let mask_codes_str = &data_str[..last_colon];
            let ip_ver_code = &data_str[last_colon + 1..];
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
                    "⬅️ 返回",
                    format!("u_kcp_done:{}", mask_codes_str),
                )],
            ];

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "🚀 <b>KCP 配置 - 批量生成</b>\n\n\
                     📋 <b>遮罩栈:</b>\n{}\n\n\
                     🌐 网络协议: <b>{}</b>\n\n\
                     ⬇️ <b>请选择生成数量:</b>",
                    stack_display.join("\n"),
                    ip_display
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
            Ok(CallbackOutcome::Done)
        }
        _ if data.starts_with("u_kcp_ok:") => {
            let data_str = data.strip_prefix("u_kcp_ok:").unwrap_or("");
            let parts: Vec<&str> = data_str.rsplitn(2, ':').collect();
            if parts.len() != 2 {
                return Ok(CallbackOutcome::Done);
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
            let ip_str = match ip_version {
                IpVersion::IPv4 => "IPv4",
                IpVersion::IPv6 => "IPv6",
                IpVersion::SplitStackV4Primary => "双栈 IPv4优先",
                IpVersion::SplitStackV6Primary => "双栈 IPv6优先",
            };

            let mask_codes: Vec<&str> = mask_codes_str.split(',').collect();

            let mask_names: Vec<&str> = mask_codes
                .iter()
                .filter_map(|c| KcpMask::from_code(c).map(|m| m.display_name()))
                .collect();
            let mask_label = mask_names.join("+");

            bot.answer_callback_query(q_id)
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
                    if !combined_links.is_empty()
                        && let Ok(msg) = bot
                            .send_message(chat_id, combined_links)
                            .parse_mode(ParseMode::Html)
                            .await
                        {
                            message_ids.push(msg.id);
                        }

                    let links_text = result.links.join("\n");
                    let mut temp_file = NamedTempFile::new()?;
                    temp_file.write_all(links_text.as_bytes())?;
                    temp_file.flush()?;
                    let file_path = temp_file.path().to_path_buf();
                    if let Ok(msg) = bot
                        .send_document(chat_id, InputFile::file(&file_path))
                        .caption(format!("KCP {} 完整链接列表", mask_label))
                        .await
                    {
                        message_ids.push(msg.id);
                    }

                    let mut result_msg = format!(
                        "✅ KCP 批量生成完成！\n\n\
                         📊 生成数量: {}\n\
                         🌐 网络协议: {}\n\
                         🎭 遮罩层: {}",
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
            Ok(CallbackOutcome::Done)
        }
        _ => Ok(CallbackOutcome::Done),
    }
}
