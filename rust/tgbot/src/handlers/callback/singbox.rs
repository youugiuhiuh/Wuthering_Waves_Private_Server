use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode};
use std::sync::Arc;
use std::time::Duration;

use crate::app::state::AppState;
use crate::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::core::types::IpVersion;

/// 安装 Sing-box
pub async fn handle_sb_install(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在安装 Sing-box...")
        .await?;

    tokio::spawn(async move {
        match SingBoxInstaller::install().await {
            Ok(_) => {
                let _ = bot.send_message(chat_id, "✅ <b>Sing-box 安装成功！</b>\n\n现在您可以创建 Hysteria2 或 TUIC 配置了。").parse_mode(ParseMode::Html).await;
            }
            Err(e) => {
                let _ = bot.send_message(chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e)).parse_mode(ParseMode::Html).await;
            }
        }
    });
    Ok(())
}

/// 初始化 Hysteria2 批量创建流程
pub async fn handle_sb_h2_init(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let has_ipv6 = crate::logic::system::SystemMonitor::get_public_ipv6().await.is_ok();
    let mut buttons = vec![vec![
        InlineKeyboardButton::callback("🌐 IPv4", "sb_h2_ip:4"),
    ]];
    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback("🌐 IPv6", "sb_h2_ip:6"));
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_singbox_mgmt")]);
    
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🚀 <b>Hysteria2 批量创建</b>\n\n请选择网络协议版本:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// 初始化 TUIC 批量创建流程
pub async fn handle_sb_tu_init(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let has_ipv6 = crate::logic::system::SystemMonitor::get_public_ipv6().await.is_ok();
    let mut buttons = vec![vec![
        InlineKeyboardButton::callback("🌐 IPv4", "sb_tu_ip:4"),
    ]];
    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback("🌐 IPv6", "sb_tu_ip:6"));
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_singbox_mgmt")]);
    
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🚀 <b>TUIC 批量创建</b>\n\n请选择网络协议版本:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// Hysteria2 IP 版本选择
pub async fn handle_sb_h2_ip(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let ip_ver = data.strip_prefix("sb_h2_ip:").unwrap_or("4");
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
    
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🚀 <b>Hysteria2 批量创建</b>\n\n🌐 网络协议版本: {}\n\n请选择生成数量:", if ip_ver == "4" { "IPv4" } else { "IPv6" }),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// Hysteria2 混淆选项
pub async fn handle_sb_h2_obfs(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.strip_prefix("sb_h2_obfs:").unwrap_or("").split(':').collect();
    if parts.len() != 2 {
        bot.answer_callback_query(q.id).text("参数错误").await?;
        return Ok(());
    }
    let ip_ver = parts[0];
    let count = parts[1];
    let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
    
    let buttons = vec![
        vec![
            InlineKeyboardButton::callback(
                "🟢 推荐：开启混淆",
                format!("sb_h2_exec:{}:{}:1", ip_ver, count),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                "🔴 不开启",
                format!("sb_h2_exec:{}:{}:0", ip_ver, count),
            ),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_h2_init")],
    ];
    
    bot.edit_message_text(
        chat_id,
        msg_id,
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
    Ok(())
}

/// TUIC IP 版本选择
pub async fn handle_sb_tu_ip(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let ip_ver = data.strip_prefix("sb_tu_ip:").unwrap_or("4");
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
    
    bot.edit_message_text(
        chat_id,
        msg_id,
        format!("🚀 <b>TUIC 批量创建</b>\n\n🌐 网络版本: {}\n\n请选择生成数量:", if ip_ver == "4" { "IPv4" } else { "IPv6" }),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

/// 批量创建 Hysteria2 配置
pub async fn handle_sb_h2_exec(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.strip_prefix("sb_h2_exec:").unwrap_or("").split(':').collect();
    if parts.len() != 3 {
        bot.answer_callback_query(q.id).text("参数错误").await?;
        return Ok(());
    }
    let ip_ver = parts[0];
    let count: usize = parts[1].parse().unwrap_or(1);
    let obfs_enabled: bool = parts[2] == "1";
    let ip_version = if ip_ver == "6" { IpVersion::IPv6 } else { IpVersion::IPv4 };
    
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在创建配置...")
        .await?;
    
    let bot_clone = bot.clone();
    let chat_id_clone = chat_id;
    
    tokio::spawn(async move {
        match SingBoxConfigManager::batch_create_hysteria2(count, ip_version, obfs_enabled).await {
            Ok(result) => {
                let mut message_ids: Vec<MessageId> = Vec::new();

                let header_msg = format!(
                    "✅ <b>Hysteria2 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
                    result.created_count,
                    result.config_file.as_deref().unwrap_or("未知")
                );
                if let Ok(msg) = bot_clone.send_message(chat_id_clone, header_msg).parse_mode(ParseMode::Html).await {
                    message_ids.push(msg.id);
                }

                let mut combined_links = String::new();
                for (i, link) in result.links.iter().enumerate() {
                    combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                    if (i + 1) % 2 == 0 {
                        if let Ok(msg) = bot_clone.send_message(chat_id_clone, combined_links.clone()).parse_mode(ParseMode::Html).await {
                            message_ids.push(msg.id);
                        }
                        combined_links.clear();
                    }
                }
                if !combined_links.is_empty() {
                    if let Ok(msg) = bot_clone.send_message(chat_id_clone, combined_links).parse_mode(ParseMode::Html).await {
                        message_ids.push(msg.id);
                    }
                }

                let links_text = result.links.join("\n");
                let timestamp = chrono::Utc::now().timestamp();
                let temp_file_path = format!("/tmp/singbox_hysteria2_links_{}.txt", timestamp);

                if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                    log::warn!("写入临时文件失败: {}", e);
                } else {
                    let doc_sent = bot_clone.send_document(chat_id_clone, InputFile::file(&temp_file_path)).caption("完整链接列表，建议尽快复制/导入").await;
                    if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                        log::warn!("删除临时文件失败: {}", e);
                    }
                    if let Ok(msg) = doc_sent {
                        message_ids.push(msg.id);
                    }
                }

                let result_msg = format!(
                    "✅ 批量创建完成！\n\n📊 生成数量: {}",
                    result.created_count
                );
                if let Ok(msg) = bot_clone.send_message(chat_id_clone, result_msg).await {
                    message_ids.push(msg.id);
                }

                let bot_clone2 = bot_clone.clone();
                let chat_id_clone2 = chat_id_clone;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    for msg_id in message_ids {
                        if let Err(e) = bot_clone2.delete_message(chat_id_clone2, msg_id).await {
                            log::warn!("删除消息失败 (chat_id: {}, msg_id: {}): {}", chat_id_clone2, msg_id, e);
                        }
                    }
                });
            }
            Err(e) => {
                let _ = bot_clone.send_message(chat_id_clone, format!("❌ <b>创建失败</b>\n原因: {}", e)).parse_mode(ParseMode::Html).await;
            }
        }
    });
    Ok(())
}

/// 批量创建 TUIC 配置
pub async fn handle_sb_tu_exec(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let parts: Vec<&str> = data.strip_prefix("sb_tu_exec:").unwrap_or("").split(':').collect();
    if parts.len() != 2 {
        bot.answer_callback_query(q.id).text("参数错误").await?;
        return Ok(());
    }
    let ip_ver = parts[0];
    let count: usize = parts[1].parse().unwrap_or(1);
    let ip_version = if ip_ver == "6" { IpVersion::IPv6 } else { IpVersion::IPv4 };
    
    bot.answer_callback_query(q.id.clone())
        .text("⏳ 正在创建配置...")
        .await?;
    
    let bot_clone = bot.clone();
    let chat_id_clone = chat_id;
    
    tokio::spawn(async move {
        match SingBoxConfigManager::batch_create_tuic(count, ip_version).await {
            Ok(result) => {
                let mut message_ids: Vec<MessageId> = Vec::new();

                let header_msg = format!(
                    "✅ <b>TUIC 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
                    result.created_count,
                    result.config_file.as_deref().unwrap_or("未知")
                );
                if let Ok(msg) = bot_clone.send_message(chat_id_clone, header_msg).parse_mode(ParseMode::Html).await {
                    message_ids.push(msg.id);
                }

                let mut combined_links = String::new();
                for (i, link) in result.links.iter().enumerate() {
                    combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                    if (i + 1) % 2 == 0 {
                        if let Ok(msg) = bot_clone.send_message(chat_id_clone, combined_links.clone()).parse_mode(ParseMode::Html).await {
                            message_ids.push(msg.id);
                        }
                        combined_links.clear();
                    }
                }
                if !combined_links.is_empty() {
                    if let Ok(msg) = bot_clone.send_message(chat_id_clone, combined_links).parse_mode(ParseMode::Html).await {
                        message_ids.push(msg.id);
                    }
                }

                let links_text = result.links.join("\n");
                let timestamp = chrono::Utc::now().timestamp();
                let temp_file_path = format!("/tmp/singbox_tuic_links_{}.txt", timestamp);

                if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                    log::warn!("写入临时文件失败: {}", e);
                } else {
                    let doc_sent = bot_clone.send_document(chat_id_clone, InputFile::file(&temp_file_path)).caption("完整链接列表，建议尽快复制/导入").await;
                    if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                        log::warn!("删除临时文件失败: {}", e);
                    }
                    if let Ok(msg) = doc_sent {
                        message_ids.push(msg.id);
                    }
                }

                let result_msg = format!(
                    "✅ 批量创建完成！\n\n📊 生成数量: {}",
                    result.created_count
                );
                if let Ok(msg) = bot_clone.send_message(chat_id_clone, result_msg).await {
                    message_ids.push(msg.id);
                }

                let bot_clone2 = bot_clone.clone();
                let chat_id_clone2 = chat_id_clone;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    for msg_id in message_ids {
                        if let Err(e) = bot_clone2.delete_message(chat_id_clone2, msg_id).await {
                            log::warn!("删除消息失败 (chat_id: {}, msg_id: {}): {}", chat_id_clone2, msg_id, e);
                        }
                    }
                });
            }
            Err(e) => {
                let _ = bot_clone.send_message(chat_id_clone, format!("❌ <b>创建失败</b>\n原因: {}", e)).parse_mode(ParseMode::Html).await;
            }
        }
    });
    Ok(())
}

/// Sing-box 删除管理菜单
pub async fn handle_sb_del_cfg(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
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
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🗑️ <b>Sing-box 删除管理</b>\n请选择删除方式 (操作不可逆):",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 删除全部配置确认
pub async fn handle_sb_del_all_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "⚠️ 确认清空所有配置 (不可恢复) ⚠️",
            "sb_del_all_exec",
        )],
        vec![InlineKeyboardButton::callback("⬅️ 取消", "sb_del_cfg")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🚨 <b>二次确认</b>\n您确定要删除 <b>所有</b> Sing-box 配置文件吗？\n此操作将清空所有配置文件、重启 Sing-box 并清理端口跳跃规则。",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行删除全部配置
pub async fn handle_sb_del_all_exec(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    match SingBoxConfigManager::delete_all_configurations().await {
        Ok(count) => {
            bot.answer_callback_query(q.id.clone())
                .text(format!("✅ 已彻底清空 {} 个 Sing-box 配置文件", count))
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

/// 按数量删除配置
pub async fn handle_sb_del_count(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
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
    bot.edit_message_text(
        chat_id,
        msg_id,
        "➗ <b>Sing-box 按数量删除 (由旧到新)</b>\n请选择要删除的文件数量:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 执行按数量删除
pub async fn handle_sb_del_exec_count(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let n: usize = data
        .strip_prefix("sb_del_exec_count:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    match SingBoxConfigManager::delete_by_count(n).await {
        Ok(deleted) => {
            bot.answer_callback_query(q.id.clone())
                .text(format!("✅ 已删除 {} 个最旧的配置文件", deleted))
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

/// 指定配置删除
pub async fn handle_sb_del_select(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
) -> ResponseResult<()> {
    let inbounds = SingBoxConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    let count = SingBoxConfigManager::get_config_count().await.unwrap_or(0);

    if inbounds.is_empty() {
        bot.answer_callback_query(q.id.clone())
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
        buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_del_cfg")]);
        bot.edit_message_text(
            chat_id,
            msg_id,
            format!(
                "🎯 <b>Sing-box 指定配置删除</b>\n\n共 {} 个配置文件，请选择要删除的:",
                count
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;
    }
    Ok(())
}

/// 执行指定配置删除
pub async fn handle_sb_del_file(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
) -> ResponseResult<()> {
    let index: usize = data
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
                bot.answer_callback_query(q.id.clone())
                    .text(format!("✅ 已删除配置文件: {}", filename))
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
            .text("❌ 文件索引无效")
            .show_alert(true)
            .await?;
    }
    Ok(())
}

/// Sing-box 回调分派
///
/// 根据 callback data 分派到对应的处理器:
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
pub async fn dispatch_callback(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: &Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        "sb_install" => handle_sb_install(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "sb_h2_init" => handle_sb_h2_init(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "sb_tu_init" => handle_sb_tu_init(bot.clone(), q.clone(), chat_id, msg_id).await?,
        d if d.starts_with("sb_h2_ip:") => {
            handle_sb_h2_ip(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("sb_tu_ip:") => {
            handle_sb_tu_ip(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("sb_h2_obfs:") => {
            handle_sb_h2_obfs(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("sb_h2_exec:") => {
            handle_sb_h2_exec(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        d if d.starts_with("sb_tu_exec:") => {
            handle_sb_tu_exec(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "sb_del_cfg" => handle_sb_del_cfg(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "sb_del_all_confirm" => {
            handle_sb_del_all_confirm(bot.clone(), q.clone(), chat_id, msg_id).await?
        }
        "sb_del_all_exec" => handle_sb_del_all_exec(bot.clone(), q.clone(), chat_id, msg_id).await?,
        "sb_del_count" => handle_sb_del_count(bot.clone(), q.clone(), chat_id, msg_id).await?,
        d if d.starts_with("sb_del_exec_count:") => {
            handle_sb_del_exec_count(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        "sb_del_select" => handle_sb_del_select(bot.clone(), q.clone(), chat_id, msg_id).await?,
        d if d.starts_with("sb_del_file:") => {
            handle_sb_del_file(bot.clone(), q.clone(), chat_id, msg_id, d).await?
        }
        _ => {}
    }
    Ok(())
}
