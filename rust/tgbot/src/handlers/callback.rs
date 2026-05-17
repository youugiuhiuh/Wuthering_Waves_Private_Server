use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::Bot;
use teloxide::prelude::{CallbackQuery, ChatId, Requester, ResponseResult};
use futures_util::future::BoxFuture;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use teloxide::payloads::{AnswerCallbackQuerySetters, EditMessageReplyMarkupSetters, EditMessageTextSetters, SendDocumentSetters, SendMessageSetters};
use tgbot::core::paths::{singbox, xray};
use tgbot::core::types::IpVersion;
use tgbot::logic;
use tgbot::logic::config::{ConfigManager, KcpMask, Proto, WarpMode};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::TaskType;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::system::SystemMonitor;
use tgbot::logic::{UpgradeManager, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use crate::app::batch_handler::send_singbox_batch_result;
use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, ScheduleFrequency, ScheduleInputState, TimeoutStatus};
use crate::bootstrap::{BotSettings, BOT_VERSION, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::utils;
use crate::utils::format_duration_human;

pub fn handle_callback(
    bot: Bot,
    mut q: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        loop {
            let user_id = q.from.id.0 as i64;
            if !state.is_authorized(user_id).await {
                bot.answer_callback_query(q.id)
                    .text("🚫 会话已过期，请发送 6 位 TOTP 验证码重新认证")
                    .await?;
                break Ok(());
            }

            let data = match q.data.as_ref() {
                Some(d) => d.clone(),
                None => break Ok(()),
            };
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

            if destruct_flow::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
                == MessageFlowOutcome::Handled
            {
                break Ok(());
            }

            let is_custom_followup = data.starts_with("s_custom_ui:")
                || data.starts_with("s_custom_set:")
                || data == "s_custom_confirm"
                || data == "s_custom_cancel";
            if is_custom_followup
                && state
                    .schedule_timeout_status(chat_id, Duration::from_secs(180))
                    .await
                    == TimeoutStatus::Expired
            {
                state.remove_schedule_input(chat_id).await;
                let new_q = q.clone();
                q = CallbackQuery {
                    data: Some("s_add_custom_menu".to_string()),
                    ..new_q
                };
                bot.answer_callback_query(q.id.clone())
                    .text("⏳ 自定义定时会话已超时，请重新进入。")
                    .show_alert(true)
                    .await?;
                continue;
            }

            if destruct_flow::handle_callback_action(
                &bot,
                &q,
                data.as_str(),
                chat_id,
                msg_id,
                &state,
            )
            .await?
                == MessageFlowOutcome::Handled
            {
                break Ok(());
            }
            // ============ 【插入分发器拦截开始】 ============
            let ctx = crate::handlers::context::CallbackContext {
                bot: bot.clone(),
                q: q.clone(),
                state: state.clone(),
                chat_id,
                msg_id,
                user_id,
                data: data.clone(),
            };

            // 1. 尝试让新模块处理，并显式捕获 anyhow 错误
            match crate::handlers::dispatch(&ctx).await {
                Ok(Some(action)) => {
                    match action {
                        crate::handlers::context::HandlerAction::Done => break Ok(()),
                        crate::handlers::context::HandlerAction::Redirect(new_data) => {
                            let new_q = q.clone();
                            q = CallbackQuery { data: Some(new_data), ..new_q };
                            continue; // 跳转逻辑
                        }
                    }
                }
                Ok(None) => {} // 未匹配到日志相关路由，安全放行给下方的老 match 块
                Err(e) => {
                    // 业务执行出错：在服务端打印错误详情
                    eprintln!("[ERROR] 运维分发器业务执行失败: {:?}", e);

                    // 给 Telegram 客户端弹窗报错，防止用户点击按钮后“死机转圈”
                    let _ = bot.answer_callback_query(q.id.clone())
                        .text("❌ 内部运维业务错误，请查看后台日志")
                        .show_alert(true)
                        .await;

                    break Ok(()); // 优雅退出，不干扰 Telegram 的更新主循环
                }
            }
            // ============ 【插入分发器拦截结束】 ============
            match data.as_str() {
                "m_xray_mgmt" => {
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    let mut buttons = Vec::new();

                    if inbounds.is_empty() {
                        buttons.push(vec![
                            InlineKeyboardButton::callback("🚀 Reality 批量备份", "u_batch_init"),
                            InlineKeyboardButton::callback(
                                "🚀 Xhttp 批量备份",
                                "u_xhttp_batch_init",
                            ),
                        ]);
                        buttons.push(vec![InlineKeyboardButton::callback(
                            "🔐 ML-DSA-65 管理",
                            "m_pq_mgmt",
                        )]);
                        bot.edit_message_text(chat_id, msg_id,
                            "🅧 <b>Xray-core 管理</b>\n\n⚠️ <b>未找到用户配置文件</b>\n\n检测到 Xray-core 已安装，但没有找到用户配置文件(*_inbounds.json)。\n\n您可以：\n• 创建 Reality 批量备份\n• 创建 Xhttp 批量备份\n• 管理 ML-DSA-65 (Reality PQ)\n• 检查配置文件是否正确放置")
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    } else {
                        // 正常显示配置文件列表
                        for (i, path) in inbounds.iter().enumerate() {
                            let filename = path.split('/').next_back().unwrap_or("Unknown");
                            buttons.push(vec![InlineKeyboardButton::callback(
                                format!("📁 {}", filename),
                                format!("u_l:{}", i),
                            )]);
                        }
                        buttons.push(vec![InlineKeyboardButton::callback(
                            "🗑️ 删除管理",
                            "m_del_cfg",
                        )]);
                        buttons.push(vec![
                            InlineKeyboardButton::callback("🚀 Reality 批量备份", "u_batch_init"),
                            InlineKeyboardButton::callback(
                                "🚀 Xhttp 批量备份",
                                "u_xhttp_batch_init",
                            ),
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
                }

                // Sing-box callbacks

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

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🚀 <b>Hysteria2 批量创建</b>\n\n请选择网络协议版本:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
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

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🚀 <b>TUIC 批量创建</b>\n\n请选择网络协议版本:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
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
                            InlineKeyboardButton::callback(
                                "10",
                                format!("sb_h2_obfs:{}:10", ip_ver),
                            ),
                            InlineKeyboardButton::callback(
                                "20",
                                format!("sb_h2_obfs:{}:20", ip_ver),
                            ),
                            InlineKeyboardButton::callback(
                                "50",
                                format!("sb_h2_obfs:{}:50", ip_ver),
                            ),
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
                }
                d if d.starts_with("sb_h2_obfs:") => {
                    let parts: Vec<&str> = d
                        .strip_prefix("sb_h2_obfs:")
                        .unwrap_or("")
                        .split(':')
                        .collect();
                    if parts.len() != 2 {
                        bot.answer_callback_query(q.id).text("参数错误").await?;
                        return Ok(());
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
                            InlineKeyboardButton::callback(
                                "10",
                                format!("sb_tu_exec:{}:10", ip_ver),
                            ),
                            InlineKeyboardButton::callback(
                                "20",
                                format!("sb_tu_exec:{}:20", ip_ver),
                            ),
                            InlineKeyboardButton::callback(
                                "50",
                                format!("sb_tu_exec:{}:50", ip_ver),
                            ),
                        ],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "sb_tu_init")],
                    ];

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!(
                            "🚀 <b>TUIC 批量创建</b>\n\n🌐 网络版本: {}\n\n请选择生成数量:",
                            if ip_ver == "4" { "IPv4" } else { "IPv6" }
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
                d if d.starts_with("sb_h2_exec:") => {
                    let parts: Vec<&str> = d
                        .strip_prefix("sb_h2_exec:")
                        .unwrap_or("")
                        .split(':')
                        .collect();
                    if parts.len() != 3 {
                        bot.answer_callback_query(q.id).text("参数错误").await?;
                        return Ok(());
                    }
                    let ip_ver = parts[0];
                    let count: usize = parts[1].parse().unwrap_or(1);
                    let obfs_enabled: bool = parts[2] == "1";
                    let ip_version = if ip_ver == "6" {
                        IpVersion::IPv6
                    } else {
                        IpVersion::IPv4
                    };

                    bot.answer_callback_query(q.id.clone())
                        .text("⏳ 正在创建配置...")
                        .await?;

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;

                    tokio::spawn(async move {
                        match SingBoxConfigManager::batch_create_hysteria2(
                            count,
                            ip_version,
                            obfs_enabled,
                        )
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
                                    .send_message(
                                        chat_id_clone,
                                        format!("❌ <b>创建失败</b>\n原因: {}", e),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }
                    });
                }
                d if d.starts_with("sb_tu_exec:") => {
                    let parts: Vec<&str> = d
                        .strip_prefix("sb_tu_exec:")
                        .unwrap_or("")
                        .split(':')
                        .collect();
                    if parts.len() != 2 {
                        bot.answer_callback_query(q.id).text("参数错误").await?;
                        return Ok(());
                    }
                    let ip_ver = parts[0];
                    let count: usize = parts[1].parse().unwrap_or(1);
                    let ip_version = if ip_ver == "6" {
                        IpVersion::IPv6
                    } else {
                        IpVersion::IPv4
                    };

                    bot.answer_callback_query(q.id.clone())
                        .text("⏳ 正在创建配置...")
                        .await?;

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;

                    tokio::spawn(async move {
                        match SingBoxConfigManager::batch_create_tuic(count, ip_version).await {
                            Ok(result) => {
                                if let Err(e) = send_singbox_batch_result(
                                    &bot_clone,
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
                                let _ = bot_clone
                                    .send_message(
                                        chat_id_clone,
                                        format!("❌ <b>创建失败</b>\n原因: {}", e),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }
                    });
                }
                // Sing-box 删除管理
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
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🗑️ <b>Sing-box 删除管理</b>\n请选择删除方式 (操作不可逆):",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "sb_del_all_confirm" => {
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
                }
                "sb_del_all_exec" => {
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("sb_del_cfg".to_string()),
                        ..new_q
                    };
                    continue;
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
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "➗ <b>Sing-box 按数量删除 (由旧到新)</b>\n请选择要删除的文件数量:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                d if d.starts_with("sb_del_exec_count:") => {
                    let n: usize = d
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("sb_del_cfg".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "sb_del_select" => {
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
                        buttons.push(vec![InlineKeyboardButton::callback(
                            "⬅️ 返回",
                            "sb_del_cfg",
                        )]);
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("sb_del_select".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "a_inst_base" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("⏳ 正在初始化 wwps 环境...")
                        .await?;

                    match RealityInstaller::run(bot.clone(), chat_id, msg_id).await {
                        Ok(outcome) => {
                            let msg = match outcome {
                                RealityInstallOutcome::Completed => {
                                    "✅ <b>wwps 环境初始化完成！</b>"
                                }
                                RealityInstallOutcome::AlreadyReady => {
                                    "✅ <b>wwps 环境已就绪。</b>"
                                }
                                RealityInstallOutcome::InProgress => {
                                    "⏳ <b>初始化正在进行中，请稍候...</b>"
                                }
                            };
                            bot.send_message(chat_id, msg)
                                .parse_mode(ParseMode::Html)
                                .await?;

                            if outcome != RealityInstallOutcome::InProgress {
                                let new_q = q.clone();
                                q = CallbackQuery {
                                    data: Some("m_users".to_string()),
                                    ..new_q
                                };
                                continue;
                            }
                        }
                        Err(e) => {
                            bot.send_message(
                                chat_id,
                                format!("❌ <b>环境初始化失败</b>\n原因: {}", e),
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                        }
                    }
}
                "a_inst_base" => {
                    if MaintenanceManager::is_reality_base_ready().await {
                        crate::show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision).await?;
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
                        crate::trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                    }
                }
                "u_xhttp_batch_init" => {
                    if MaintenanceManager::is_reality_base_ready().await {
                        crate::show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::XHTTP).await?;
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
                        crate::trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                    }
                }
                d if d.starts_with("u_batch_ip_init:")
                    || d.starts_with("u_xhttp_batch_ip_init:") =>
                {
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
                    // 进入第二步：选择数量
                    crate::show_reality_qty_prompt(&bot, chat_id, msg_id, ip_version, proto).await?;
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
                    let ip_ver_code = parts[0]; // "4" / "6" / "s6" / "s4"
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
                        crate::trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
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
                            // 收集所有消息 ID，用于后续自动删除
                            let mut message_ids: Vec<MessageId> = Vec::new();

                            // 发送链接（每条消息包含 2 条链接）
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
                            let temp_path = temp_file.into_temp_path();
                            let file_path = PathBuf::from(temp_path.as_os_str());
                            if let Ok(msg) = bot
                                .send_document(chat_id, InputFile::file(&file_path))
                                .caption("完整链接列表，建议尽快复制/导入")
                                .await
                            {
                                message_ids.push(msg.id);
                            }

                            // 发送结果信息
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

                            // 启动后台任务，60秒后自动删除所有消息
                            let bot_clone = bot.clone();
                            let chat_id_clone = chat_id;
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(60)).await;
                                for msg_id in message_ids {
                                    if let Err(e) =
                                        bot_clone.delete_message(chat_id_clone, msg_id).await
                                    {
                                        log::warn!(
                                            "删除消息失败 (chat_id: {}, msg_id: {}): {}",
                                            chat_id_clone,
                                            msg_id,
                                            e
                                        );
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            if err_msg.contains("未找到 Reality 配置文件") {
                                bot.send_message(
                                    chat_id,
                                    "⚠️ <b>检测到 Reality 母版缺失，正在自动初始化...</b>",
                                )
                                .parse_mode(ParseMode::Html)
                                .await?;
                                crate::trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                            } else {
                                bot.send_message(chat_id, format!("❌ 生成失败: {}", err_msg))
                                    .await?;
                            }
                        }
                    }
                }
                // ==================== KCP Handlers (Category Navigation UI) ====================
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
                }
                d if d.starts_with("u_kcp_more:") => {
                    let existing = d.strip_prefix("u_kcp_more:").unwrap_or("");
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
                        (
                            "enc",
                            "🔐 加密层",
                            KcpMask::variants_by_category("enc").len(),
                        ),
                        (
                            "obf",
                            "🌀 混淆层",
                            KcpMask::variants_by_category("obf").len(),
                        ),
                        (
                            "dis",
                            "🎭 伪装层",
                            KcpMask::variants_by_category("dis").len(),
                        ),
                        (
                            "ext",
                            "⚡ 扩展层",
                            KcpMask::variants_by_category("ext").len(),
                        ),
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

                    let masks: Vec<KcpMask> =
                        codes.iter().filter_map(|c| KcpMask::from_code(c)).collect();

                    let ordered = KcpMask::canonical_order(&masks);

                    if let Err(e) = KcpMask::validate_stack(&ordered) {
                        bot.answer_callback_query(q.id.clone())
                            .text(format!("❌ {}", e))
                            .await?;
                        return Ok(());
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
                }
                d if d.starts_with("u_kcp_ip:") => {
                    let data = d.strip_prefix("u_kcp_ip:").unwrap_or("");
                    let last_colon = data.rfind(':').unwrap_or(data.len());
                    let mask_codes_str = &data[..last_colon];
                    let ip_ver_code = &data[last_colon + 1..];
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
                            let temp_path = temp_file.into_temp_path();
                            let file_path = PathBuf::from(temp_path.as_os_str());
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
                }
                // 用户列表
                d if d.starts_with("u_l:") => {
                    let idx: usize = d.strip_prefix("u_l:").unwrap_or("0").parse().unwrap_or(0);
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    if let Err(e) = utils::validate_idx(idx, inbounds.len(), "用户配置") {
                        bot.answer_callback_query(q.id.clone())
                            .text(format!("❌ {}", e))
                            .await?;
                        continue;
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
                // 删除特定用户逻辑
                // 删除特定用户逻辑
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
                            format!("⚠️ <b>删除确认</b>\n\n您确定要删除用户 <code>{}</code> 吗？\n(注意：当前版本暂未实现单个用户删除逻辑，此操作可能仅用于演示 UI)", utils::escape_html(email))
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
                        // TODO: call actual delete logic
                        bot.answer_callback_query(q.id.clone())
                            .text(format!("🗑 暂不支持删除单个用户: {}", email))
                            .show_alert(true)
                            .await?;
                    }
                }
                d if d.starts_with("cfg_filter:") => {
                    let filter = d.strip_prefix("cfg_filter:").unwrap_or("all");
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
                }
                "m_del_cfg" => {
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
                }
                "m_pq_del" => {
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("m_pq_mgmt".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "m_pq_init" => {
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("m_pq_mgmt".to_string()),
                        ..new_q
                    };
                    continue;
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
                        format!("🚨 <b>二次确认</b>\n您确定要删除 <b>{}</b> 类型的所有配置文件吗？\n此操作将清空相关 batch_* 文件并重启核心。", filter_type_label),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
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
                                bot.answer_callback_query(q.id.clone())
                                    .text("❌ 未知筛选类型")
                                    .await?;
                                let new_q = q.clone();
                                q = CallbackQuery {
                                    data: Some("m_del_cfg".to_string()),
                                    ..new_q
                                };
                                continue;
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
                            let _ =
                                crate::logic::maintenance::MaintenanceManager::reload_core().await;
                        }
                        count
                    };
                    bot.answer_callback_query(q.id.clone())
                        .text(format!("✅ 已彻底清空 {} 个配置文件", count))
                        .show_alert(true)
                        .await?;
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("m_del_cfg".to_string()),
                        ..new_q
                    };
                    continue;
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
                            InlineKeyboardButton::callback(
                                "10 个",
                                format!("cfg_del_exec_count:{}:10", filter),
                            ),
                            InlineKeyboardButton::callback(
                                "50 个",
                                format!("cfg_del_exec_count:{}:50", filter),
                            ),
                        ],
                        vec![
                            InlineKeyboardButton::callback(
                                "100 个",
                                format!("cfg_del_exec_count:{}:100", filter),
                            ),
                            InlineKeyboardButton::callback(
                                "500 个",
                                format!("cfg_del_exec_count:{}:500", filter),
                            ),
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
                }
                d if d.starts_with("cfg_del_exec_count:") => {
                    let parts: Vec<&str> = d.split(':').collect();
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
                        let _ = crate::logic::maintenance::MaintenanceManager::reload_core().await;
                    }
                    bot.answer_callback_query(q.id.clone())
                        .text(format!("✅ 已成功清理 {} 个旧配置", deleted_count))
                        .show_alert(true)
                        .await?;
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some(format!("cfg_del_count:{}", filter)),
                        ..new_q
                    };
                    continue;
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
                }
                d if d.starts_with("cfg_del_file:") => {
                    let parts: Vec<&str> = d.split(':').collect();
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
                            format!("⚠️ <b>删除确认</b>\n\n您确定要删除配置文件 <code>{}</code> 吗？\n此操作不可恢复！", utils::escape_html(filename)),
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("❌ 文件不存在或已被删除")
                            .await?;
                    }
                }
                d if d.starts_with("cfg_del_confirm:") => {
                    let parts: Vec<&str> = d.split(':').collect();
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

                    if let Err(e) = utils::validate_idx(idx, files.len(), "配置文件") {
                        bot.answer_callback_query(q.id.clone())
                            .text(format!("❌ {}", e))
                            .await?;
                        continue;
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some(format!("cfg_del_select:{}", filter)),
                        ..new_q
                    };
                    continue;
                }
                _ => {
                    bot.answer_callback_query(q.id).await?;
                }
            }
            break Ok(());
        }
    })
}