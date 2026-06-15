use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::bootstrap::{BOT_VERSION, BotSettings, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::utils::format_duration_human;
use aegis::adapters::common::TargetId;
use aegis::core::paths::{singbox, xray};
use aegis::core::singbox::SingBoxInstaller;
use aegis::core::system::SystemMonitor;
use aegis::core::system::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use std::path::Path;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📊 系统状态", "m_mon"),
            InlineKeyboardButton::callback("👥 用户管理", "m_usr"),
        ],
        vec![InlineKeyboardButton::callback(
            "🛠 运维中心 (Ops)",
            "m_ops_center",
        )],
        vec![InlineKeyboardButton::callback("⚙️ 系统设置", "m_settings")],
    ]);
    bot.send_message(chat_id, "🏠 <b>主菜单</b>\n请选择操作类目:")
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "m_main" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("📊 状态监控", "m_mon"),
                    InlineKeyboardButton::callback("👥 用户管理", "m_usr"),
                ],
                vec![
                    InlineKeyboardButton::callback("🛠 运维中心", "m_ops_center"),
                    InlineKeyboardButton::callback("⚙️ 系统设置", "m_settings"),
                ],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, "🏠 <b>主菜单</b>\n请选择功能模块:")
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_ops_center" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🌩 网络优化", "m_net_opt"),
                    InlineKeyboardButton::callback("🛡 安全防护", "m_security"),
                ],
                vec![
                    InlineKeyboardButton::callback("💻 系统指令", "m_sys_cmd"),
                    InlineKeyboardButton::callback("📄 日志审计", "m_log"),
                ],
                vec![InlineKeyboardButton::callback("⬅️ 返回主菜单", "m_main")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🛠 <b>运维中心</b>\n集成网络、安全及系统管理工具:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_settings" => {
            let timeout = ctx.state.session_timeout_secs().await;
            let timeout_label = format!("🔐 会话有效期 ({})", format_duration_human(timeout));
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🛰 Xray-core 管理", "a_wwps_core_menu"),
                    InlineKeyboardButton::callback("📦 Sing-box 管理", "a_wwps_box_menu"),
                ],
                vec![InlineKeyboardButton::callback("⏰ 定时任务", "m_sched")],
                vec![
                    InlineKeyboardButton::callback("🌍 Geo数据", "a_geo_menu"),
                    InlineKeyboardButton::callback("⚙️ Bot更新", "a_upgrade"),
                ],
                vec![InlineKeyboardButton::callback(
                    &timeout_label,
                    "m_session_timeout",
                )],
                vec![InlineKeyboardButton::callback("⚠️ 危险区域", "m_danger")],
                vec![InlineKeyboardButton::callback("⬅️ 返回主菜单", "m_main")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "⚙️ <b>系统设置</b>\n管理核心版本、任务调度及数据更新:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_net_opt" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🌩 WARP 分流", "m_warp"),
                    InlineKeyboardButton::callback("🚀 BBR3 + 通用优化", "a_bbr3"),
                ],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回运维中心",
                    "m_ops_center",
                )],
            ]);
            ctx.bot.edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            "🌩 <b>网络优化</b>\n选择优化方案:\n\n<code>BBR3 + 通用优化</code> 会同时处理内核安装与 sysctl 调优。",
        )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
        }
        "m_security" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback("🛡 防火墙加固", "a_fw")],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回运维中心",
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, "🛡 <b>安全防护</b>\n系统安全配置:")
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_sys_cmd" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🔄 重启系统", "a_sys_reboot"),
                    InlineKeyboardButton::callback("♻️ 重启核心", "a_reload"),
                ],
                vec![InlineKeyboardButton::callback(
                    "⚙️ 配置自动更新",
                    "a_sys_maint",
                )],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回运维中心",
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "💻 <b>系统指令</b>\n执行系统级操作:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_geo_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback("🔄 立即更新", "a_geo")],
                vec![InlineKeyboardButton::callback(
                    "⏰ 自动调度",
                    "a_geo_sched_menu",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🌍 <b>Geo数据管理</b>\n管理 GeoIP/GeoSite 数据库:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_mon" => {
            let report = SystemMonitor::get_status_report()
                .await
                .unwrap_or_else(|e| format!("❌ 获取状态失败: {}", e));
            let (wwps_core, wwps_box) = SystemMonitor::get_core_status().await;

            let status_text = format!(
                "{}\n\n🤖 <b>Bot 版本</b>: v{}\n\n⚙️ <b>核心进程</b>:\n- Xray-core: {}\n- Sing-box: {}",
                report,
                BOT_VERSION,
                if wwps_core { "🟢" } else { "🔴" },
                if wwps_box { "🟢" } else { "🔴" }
            );

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback("🔄 刷新", "m_mon")],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, status_text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_usr" => {
            let wwps_core_config_exists = Path::new(xray::CONF_DIR).exists();
            let singbox_config_exists = Path::new(singbox::CONF_DIR).exists();
            let mut buttons = Vec::new();

            if !wwps_core_config_exists && !singbox_config_exists {
                buttons.push(vec![InlineKeyboardButton::callback(
                    "🚀 初始化 wwps 环境",
                    "a_inst_base",
                )]);
                ctx.bot.edit_message_text(ctx.chat_id, ctx.msg_id,
                    "👥 <b>用户管理</b>\n\n❌ <b>未检测到 wwps 配置</b>\n\n当前系统尚未安装 wwps 或配置目录不存在。\n\n请先安装并配置 wwps 后再使用用户管理功能。")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else {
                buttons.push(vec![InlineKeyboardButton::callback(
                    "🅧 Xray-core 管理",
                    "m_xray_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    "📦 Sing-box 管理",
                    "m_singbox_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        "👥 <b>用户管理</b>\n\n请选择核心类型:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }
        }
        "m_session_timeout" => {
            let current = ctx.state.session_timeout_secs().await;
            let options: Vec<(u64, &str)> = vec![
                (5 * 60, "5分钟"),
                (10 * 60, "10分钟"),
                (30 * 60, "30分钟"),
                (60 * 60, "1小时"),
                (4 * 3600, "4小时"),
                (12 * 3600, "12小时"),
                (24 * 3600, "24小时"),
            ];
            let mut rows = Vec::new();
            for chunk in options.chunks(3) {
                let row: Vec<InlineKeyboardButton> = chunk
                    .iter()
                    .map(|(secs, label)| {
                        let prefix = if *secs == current { "✅ " } else { "" };
                        InlineKeyboardButton::callback(
                            format!("{}{}", prefix, label),
                            format!("set_timeout:{}", secs),
                        )
                    })
                    .collect();
                rows.push(row);
            }
            rows.push(vec![InlineKeyboardButton::callback(
                "⬅️ 返回设置",
                "m_settings",
            )]);

            ctx.bot.edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            format!(
                "🔐 <b>会话有效期设置</b>\n\n当前: <b>{}</b>\n\nTOTP 认证后的会话有效时长，过期需重新认证。",
                format_duration_human(current)
            ),
        )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(rows))
            .await?;
        }
        d if d.starts_with("set_timeout:") => {
            let secs: u64 = d
                .strip_prefix("set_timeout:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS);
            ctx.state.set_session_timeout_secs(secs).await;
            let settings = BotSettings {
                session_timeout_secs: secs,
            };
            if let Err(e) = settings.save() {
                log::error!("保存会话设置失败: {}", e);
            }
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(format!(
                    "✅ 会话有效期已设为 {}",
                    format_duration_human(secs)
                ))
                .await?;

            return Ok(HandlerAction::Redirect("m_session_timeout".to_string()));
        }
        "m_danger" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "💥 立即自毁 (VPS过期一键删)",
                    "a_destroy_ask",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "⚠️ <b>危险区域</b>\n\n此处包含不可逆的破坏性操作。\n请谨慎操作！",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "🔄 更新到最新 (默认)",
                    "a_wwps_core_latest",
                )],
                vec![InlineKeyboardButton::callback(
                    "📜 选择版本 (最近 5 个)",
                    "a_wwps_core_tags",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🛰️ <b>wwps-core 管理</b>\n默认更新到最新版本，或选择指定版本。",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_latest" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("🛰️ 正在启动 wwps-core 升级 (最新版本)...")
                .await?;
            let adapter = ctx.state.adapter.clone();
            let target = TargetId(ctx.chat_id.0.to_string());
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(None, adapter.as_ref(), &target).await
                {
                    let _ = bot_clone
                        .send_message(chat_id_clone, format!("❌ wwps-core 升级失败: {}", err))
                        .await;
                }
            });
        }
        "a_wwps_core_tags" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("📜 正在获取最近 5 个版本...")
                .await?;

            let reply =
                match WwpsCoreUpgradeConfig::from_env().and_then(WwpsCoreUpgradeManager::new) {
                    Ok(manager) => match manager.fetch_recent_tags(5).await {
                        Ok(tags) if !tags.is_empty() => {
                            let mut buttons = Vec::new();
                            for tag in tags {
                                buttons.push(vec![InlineKeyboardButton::callback(
                                    format!("⬆️ {}", tag),
                                    format!("wwps_core_tag:{}", tag),
                                )]);
                            }
                            buttons.push(vec![InlineKeyboardButton::callback(
                                "⬅️ 返回",
                                "a_wwps_core_menu",
                            )]);
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    "请选择要安装的 wwps-core 版本：",
                                )
                                .reply_markup(InlineKeyboardMarkup::new(buttons))
                                .await
                        }
                        Ok(_) => {
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    "未获取到可用版本，请稍后重试。",
                                )
                                .await
                        }
                        Err(err) => {
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    format!("❌ 获取版本列表失败: {}", err),
                                )
                                .await
                        }
                    },
                    Err(err) => {
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                format!("❌ wwps-core 配置错误: {}", err),
                            )
                            .await
                    }
                };

            if reply.is_err() {
                let _ = ctx
                    .bot
                    .send_message(
                        ctx.chat_id,
                        "❌ 无法获取版本列表，请检查网络或 GitHub 访问。",
                    )
                    .await;
            }
        }
        d if d.starts_with("wwps_core_tag:") => {
            let tag = d.strip_prefix("wwps_core_tag:").unwrap_or("").to_string();
            if tag.is_empty() {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("❌ 版本信息为空")
                    .await?;
                return Ok(HandlerAction::Done);
            }

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(format!("🛰️ 正在升级到版本 {}...", tag))
                .await?;

            let adapter = ctx.state.adapter.clone();
            let target = TargetId(ctx.chat_id.0.to_string());
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(Some(tag), adapter.as_ref(), &target).await
                {
                    let _ = bot_clone
                        .send_message(chat_id_clone, format!("❌ wwps-core 升级失败: {}", err))
                        .await;
                }
            });
        }
        "a_wwps_box_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "🔄 重启服务",
                    "a_wwps_box_restart",
                )],
                vec![InlineKeyboardButton::callback(
                    "📊 查看状态",
                    "a_wwps_box_status",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "📦 <b>Sing-box 管理</b>\n管理 Sing-box 服务状态",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_box_restart" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("🔄 正在重启 Sing-box 服务...")
                .await?;

            match SingBoxInstaller::restart_service().await {
                Ok(_) => {
                    ctx.bot
                        .edit_message_text(ctx.chat_id, ctx.msg_id, "✅ <b>Sing-box 重启成功</b>")
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(ctx.chat_id, ctx.msg_id, format!("❌ 重启失败: {}", err))
                        .await?;
                }
            }
        }
        "a_wwps_box_status" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("📊 正在获取状态...")
                .await?;

            match SingBoxInstaller::status().await {
                Ok(status) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            format!("📦 <b>Sing-box 状态</b>\n\n{}", status),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            format!("❌ 获取状态失败: {}", err),
                        )
                        .await?;
                }
            }
        }
        _ => {}
    }
    Ok(HandlerAction::Done)
}
