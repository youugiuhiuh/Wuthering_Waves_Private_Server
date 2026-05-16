// mod logic; // Moved to lib.rs
#![allow(clippy::vec_init_then_push)]
mod app;
mod bootstrap;
mod handlers;

use obfstr::obfstr;

use tgbot::logic;

use crate::app::auth;
use crate::app::{destruct_flow::{self, MessageFlowOutcome}, state::{AppState, TimeoutStatus}};
use crate::handlers::schedule::handle_schedule_callback;
use crate::handlers::xray_config::handle_xray_config_callback;
use crate::handlers::CallbackOutcome;
use crate::bootstrap::{
    BOT_VERSION, BotSettings, CONFIG_FILE, ConfigValidator,
    EncryptedConfig, KEY_FILE, config_dir, harden_process, run_setup, run_setup_from_stdin,
    verify_integrity,
};
use crate::handlers::utils::*;
use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, ParseMode,
};
use teloxide::utils::command::BotCommands;
use tgbot::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use tgbot::core::paths::{singbox, xray};
use tgbot::logic::bot_upgrade::{UPGRADE_FLAG_FILE, UpgradeManager};
use tgbot::logic::config::ConfigManager;
use tgbot::logic::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::task_types::TaskType;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::self_destruct::production_executor;
use tgbot::logic::singbox::SingBoxInstaller;
use tgbot::logic::system::SystemMonitor;
use tgbot::logic::totp::TotpManager;
// TOTP 防爆破参数
// TOTP 防爆破参数
const TOTP_FAIL_MAX: u32 = 5; // 窗口内最大失败次数
const TOTP_FAIL_WINDOW: Duration = Duration::from_secs(10 * 60); // 10 分钟

// 递增锁定策略: 15m -> 1h -> 24h -> 48h
const LOCKOUT_DURATIONS: [Duration; 4] = [
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(24 * 60 * 60),
    Duration::from_secs(48 * 60 * 60),
];

async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context(obfstr!("无法向 Telegram 注册主命令").to_string())?;
    println!("{}", obfstr!("✅ 已向 Telegram 注册主命令"));
    Ok(())
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "支持以下命令:")]
enum Command {
    #[command(description = "显示帮助信息")]
    Help,
    #[command(description = "启动机器人")]
    Start,
    #[command(description = "显示管理菜单")]
    Menu,
    #[command(description = "验证 TOTP 认证码")]
    Auth(String),
    #[command(description = "设置自毁验证文件 (需附带文件)")]
    SetSecurityFile,
}

fn looks_like_totp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

async fn process_auth_code(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    code: &str,
    state: &Arc<AppState>,
) -> ResponseResult<bool> {
    auth::process_auth_code(
        bot,
        chat_id,
        user_id,
        code,
        state,
        TOTP_FAIL_MAX,
        TOTP_FAIL_WINDOW,
        &LOCKOUT_DURATIONS,
    )
    .await
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(msg.chat.id, "⚠️ 无法识别用户身份，请访问管理员检查权限")
            .await?;
        return Ok(());
    };
    let user_id = from.id.0 as i64;

    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "👋 欢迎使用 wwps 管理机器人！

请发送 6 位 TOTP 验证码（或使用 /auth <验证码>）解锁 24 小时管理权限。",
            )
            .await?;
        }
        Command::Auth(code) => {
            let _ = process_auth_code(&bot, msg.chat.id, user_id, &code, &state).await?;
        }
        Command::SetSecurityFile => {
            if !state.is_recently_authenticated(user_id).await {
                bot.send_message(
                    msg.chat.id,
                    "⚠️ 此操作需要重新认证。请先发送 TOTP 验证码（或 /auth <验证码>）进行认证，5 分钟内再试。",
                )
                .await?;
                return Ok(());
            }

            // Check for document or photo on current message or replied message
            let file_id = msg
                .document()
                .map(|doc| doc.file.id.clone())
                .or_else(|| {
                    msg.photo()
                        .and_then(|photos| photos.last().map(|p| p.file.id.clone()))
                })
                .or_else(|| {
                    msg.reply_to_message().and_then(|reply| {
                        reply.document().map(|doc| doc.file.id.clone()).or_else(|| {
                            reply
                                .photo()
                                .and_then(|photos| photos.last().map(|p| p.file.id.clone()))
                        })
                    })
                });

            if let Some(fid) = file_id {
                let file = bot.get_file(fid.clone()).await?;

                if file.size as u64 > MAX_FILE_DOWNLOAD_SIZE {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ 文件过大 ({} bytes)，最大允许 {} bytes",
                            file.size, MAX_FILE_DOWNLOAD_SIZE
                        ),
                    )
                    .await?;
                    return Ok(());
                }

                let mut content = Vec::new();
                bot.download_file(&file.path, &mut content)
                    .await
                    .map_err(std::io::Error::other)?;

                // Calculate SHA-256
                let mut hasher = Sha256::new();
                hasher.update(&content);
                let result = hasher.finalize();
                let hash_hex = hex::encode(result);

                // Update state
                state
                    .set_self_destruct_key_hash(Some(hash_hex.clone()))
                    .await;

                // Save config
                save_config(&state).await.map_err(std::io::Error::other)?;

                bot.send_message(
                    msg.chat.id,
                    format!("✅ 安全验证文件已设置。\nHash: `{}`", hash_hex),
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            } else {
                bot.send_message(
                    msg.chat.id,
                    "⚠️ 请发送一个文件或图片，并附带 caption `/setsecurityfile`，或者回复该命令到文件消息。",
                ).await?;
            }
        }
        Command::Menu => {
            if !state.is_authorized(user_id).await {
                bot.send_message(
                    msg.chat.id,
                    "🔐 请先发送 6 位 TOTP 验证码进行认证（或 /auth <验证码>）。",
                )
                .await?;
                return Ok(());
            }
            send_main_menu(bot, msg.chat.id).await?;
        }
    }

    Ok(())
}

const MAX_INPUT_LENGTH: usize = 4096;
const MAX_FILE_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024;

async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, "⚠️ 无法识别用户身份，请访问管理员检查权限")
            .await?;
        return Ok(());
    };
    let user_id = from.id.0 as i64;

    if !state.is_admin_user(user_id) {
        return Ok(());
    }

    if let Some(text) = msg.text()
        && text.len() > MAX_INPUT_LENGTH
    {
        bot.send_message(
            chat_id,
            format!("⚠️ 输入过长，请控制在 {} 字符以内。", MAX_INPUT_LENGTH),
        )
        .await?;
        return Ok(());
    }

    match state
        .schedule_timeout_status(chat_id, Duration::from_secs(180))
        .await
    {
        TimeoutStatus::Expired => {
            state.remove_schedule_input(chat_id).await;
            bot.send_message(chat_id, "⏳ 定时任务选择超时 (180s)，已自动取消。")
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if msg.text().is_some() || msg.document().is_some() || msg.photo().is_some() {
                bot.send_message(
                    chat_id,
                    "ℹ️ 请通过面板按钮选择 星期/小时/分钟，然后点击“确认创建任务”。",
                )
                .await?;
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    match state
        .take_warp_input_status(chat_id, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            bot.send_message(chat_id, "⏳ 输入超时 (60s)，已自动取消。")
                .await?;
            return Ok(());
        }
        TimeoutStatus::Active => {
            if let Some(text) = msg.text() {
                let rules: Vec<String> = text
                    .split([',', '，', '\n'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if rules.is_empty() {
                    bot.send_message(chat_id, "⚠️ 输入为空，请重新输入或使用 /menu 返回。")
                        .await?;
                    return Ok(());
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        bot.send_message(chat_id, "✅ WARP 分流规则已添加并重载核心。")
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ 添加失败: {}", e))
                            .await?;
                    }
                }
            }
            return Ok(());
        }
        TimeoutStatus::NotTracked => {}
    }

    if destruct_flow::handle_message_flow(&bot, &msg, user_id, &state).await?
        == MessageFlowOutcome::Handled
    {
        return Ok(());
    }

    if let Some(text) = msg.text() {
        let code = text.trim();
        if looks_like_totp_code(code) && !state.is_authorized(user_id).await {
            let _ = process_auth_code(&bot, chat_id, user_id, code, &state).await?;
            return Ok(());
        }
    }

    Ok(())
}

async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
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

fn handle_callback(
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

            let mut data = match q.data.as_ref() {
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

            match data.as_str() {
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
                    bot.edit_message_text(chat_id, msg_id, "🏠 <b>主菜单</b>\n请选择功能模块:")
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
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🛠 <b>运维中心</b>\n集成网络、安全及系统管理工具:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "m_settings" => {
                    let timeout = state.session_timeout_secs().await;
                    let timeout_label =
                        format!("🔐 会话有效期 ({})", format_duration_human(timeout));
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
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
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
                    bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "🌩 <b>网络优化</b>\n选择优化方案:\n\n<code>BBR3 + 通用优化</code> 会同时处理内核安装与 sysctl 调优。",
                )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "m_security" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback("🛡 防火墙加固", "a_fw")],
                        // Future: Add Fail2ban check etc.
                        vec![InlineKeyboardButton::callback(
                            "⬅️ 返回运维中心",
                            "m_ops_center",
                        )],
                    ]);
                    bot.edit_message_text(chat_id, msg_id, "🛡 <b>安全防护</b>\n系统安全配置:")
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
                    bot.edit_message_text(chat_id, msg_id, "💻 <b>系统指令</b>\n执行系统级操作:")
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
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
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
                    // get_core_status is still useful
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
                    bot.edit_message_text(chat_id, msg_id, status_text)
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
                        bot.edit_message_text(chat_id, msg_id,
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
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "👥 <b>用户管理</b>\n\n请选择核心类型:",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    }
                }
// ==================== XRay Config Handler ====================
                d if d.starts_with("m_xray_mgmt") || d.starts_with("u_l:") || d.starts_with("u_d:") || d.starts_with("u_d_confirm:") || d.starts_with("cfg_filter:") || d.starts_with("m_del_cfg") || d.starts_with("m_pq_mgmt") || d.starts_with("m_pq_del") || d.starts_with("m_pq_init") || d == "cfg_del_all_confirm" || d.starts_with("cfg_del_all_confirm:") || d == "cfg_del_all_exec" || d.starts_with("cfg_del_all_exec:") || d == "cfg_del_count" || d.starts_with("cfg_del_count:") || d.starts_with("cfg_del_exec_count:") || d == "cfg_del_select" || d.starts_with("cfg_del_select:") || d.starts_with("cfg_del_file:") || d.starts_with("cfg_del_confirm:") => {
                    match handle_xray_config_callback(bot.clone(), chat_id, msg_id, d, q.id.as_str(), state.clone()).await {
                        Ok(CallbackOutcome::Done) => break Ok(()),
                        Ok(CallbackOutcome::Redirect(new_data)) => { data = new_data; continue; }
                        Err(e) => {
                            log::warn!("XRay config handler error: {}", e);
                            break Ok(());
                        }
                    }
                }
                "a_reload" => {
                    let _ = MaintenanceManager::reload_core().await;
                    bot.answer_callback_query(q.id)
                        .text("✅ 已重启核心")
                        .await?;
                }
                "a_fw" => {
                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

                    tokio::spawn(async move {
                        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                        // 进度更新任务 (带节流)
                        let bot_for_updates = bot_clone.clone();
                        let update_task = tokio::spawn(async move {
                            let mut last_text = String::new();
                            while let Some(text) = rx.recv().await {
                                if text == last_text {
                                    continue;
                                }
                                last_text = text.clone();
                                let _ = bot_for_updates
                                    .edit_message_text(
                                        chat_id_clone,
                                        msg_id,
                                        format!("🛡️ <b>防火墙安全加固</b>\n{}", text),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                                // 强制等待 500ms，避免 Telegram 频率限制
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            }
                        });

                        let tx_clone = tx.clone();
                        let res_timeout = tokio::time::timeout(
                            tokio::time::Duration::from_secs(45), // 45秒超时
                            MaintenanceManager::harden_firewall(move |text| {
                                let _ = tx_clone.send(text.to_string());
                            }),
                        )
                        .await;

                        match res_timeout {
                            Ok(Ok(_)) => {
                                // 正常结束，update_task 会在 rx 关闭后退出
                            }
                            Ok(Err(err)) => {
                                let _ = tx.send(format!("❌ 失败: {}", err));
                            }
                            Err(_) => {
                                let _ = tx.send(
                                    "❌ 失败: 操作超时 (45s)，请检查系统 nftables 状态".to_string(),
                                );
                            }
                        }

                        drop(tx); // 关闭 channel 触发 update_task 退出
                        let _ = update_task.await;
                    });

                    bot.answer_callback_query(q.id)
                        .text("⚙️ 正在启动防火墙扫描与加固...")
                        .await?;
                }
                "a_upgrade" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("⚙️ 正在启动 Bot 自更新...")
                        .await?;
                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    tokio::spawn(async move {
                        match UpgradeManager::new() {
                            Ok(manager) => {
                                if let Err(err) =
                                    manager.run(bot_clone.clone(), chat_id_clone).await
                                {
                                    let _ = bot_clone
                                        .send_message(
                                            chat_id_clone,
                                            format!("❌ 自更新失败: {}", err),
                                        )
                                        .await;
                                }
                            }
                            Err(err) => {
                                let _ = bot_clone
                                    .send_message(
                                        chat_id_clone,
                                        format!("❌ 无法启动自更新: {}", err),
                                    )
                                    .await;
                            }
                        }
                    });
                }
                "a_geo" => {
                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    let msg_id_clone = msg_id;

                    tokio::spawn(async move {
                        let bot_for_cb = bot_clone.clone();
                        let progress_cb = move |_: f64, text: &str| {
                            let bot = bot_for_cb.clone();
                            let text = text.to_string();
                            tokio::spawn(async move {
                                let _ = bot
                                    .edit_message_text(
                                        chat_id_clone,
                                        msg_id_clone,
                                        format!("🌍 <b>GeoData 更新中</b>\n{}", text),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            });
                        };

                        match MaintenanceManager::update_geodata(progress_cb).await {
                            Ok(_) => {
                                let _ = bot_clone
                                    .send_message(chat_id_clone, "✅ GeoData 更新成功")
                                    .await;
                            }
                            Err(e) => {
                                let _ = bot_clone
                                    .send_message(
                                        chat_id_clone,
                                        format!("❌ GeoData 更新失败: {}", e),
                                    )
                                    .await;
                            }
                        }
                    });

                    bot.answer_callback_query(q.id)
                        .text("🌍 GeoData 已启动更新 (后台执行)")
                        .await?;
                }
                "a_geo_sched_menu" => {
                    let geo_info = if let Some(manager) = logic::scheduler::get_manager().await {
                        let s = manager.state.lock().await;
                        let geo_tasks: Vec<_> = s
                            .tasks
                            .iter()
                            .filter(|t| t.task_type == TaskType::GeoUpdate)
                            .collect();
                        if geo_tasks.is_empty() {
                            "📝 当前无 Geo 自动更新任务".to_string()
                        } else {
                            let mut info = "⏰ <b>当前 Geo 定时任务</b>:\n\n".to_string();
                            for (i, t) in geo_tasks.iter().enumerate() {
                                info.push_str(&format!(
                                    "{}. Cron: <code>{}</code> | TZ: <code>{}</code>\n",
                                    i + 1,
                                    t.cron_expression,
                                    t.timezone
                                ));
                            }
                            info
                        }
                    } else {
                        "❌ 调度器未初始化".to_string()
                    };

                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::callback("🟢 每天", "s_custom:geo:daily"),
                            InlineKeyboardButton::callback("🟢 每周", "s_custom:geo:weekly"),
                        ],
                        vec![InlineKeyboardButton::callback(
                            "⛔️ 停止 Geo 自动更新",
                            "geo_sched_off",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "⬅️ 返回 Geo 数据",
                            "a_geo_menu",
                        )],
                    ]);

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!(
                            "🌍 <b>Geo 自动更新调度</b>\n\n{}\n\n选择周期来自定义调度时间:",
                            geo_info
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "geo_sched_off" => {
                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let mut state_lock = manager.state.lock().await;
                        let mut removed = false;
                        for i in (0..state_lock.tasks.len()).rev() {
                            if state_lock.tasks[i].task_type == TaskType::GeoUpdate {
                                state_lock.tasks.remove(i);
                                removed = true;
                            }
                        }
                        let _ = state_lock.save_to_file(&manager.state_path);
                        drop(state_lock);
                        let _ = manager.start_all_tasks(bot.clone(), state.admin_id()).await;

                        bot.answer_callback_query(q.id.clone())
                            .text(if removed {
                                "✅ 已停止 Geo 自动更新"
                            } else {
                                "ℹ️ 未找到 Geo 自动更新任务"
                            })
                            .await?;

                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some("a_geo_sched_menu".to_string()),
                            ..new_q
                        };
                        continue;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("❌ 调度器未初始化")
                            .await?;
                    }
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

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🛰️ <b>wwps-core 管理</b>\n默认更新到最新版本，或选择指定版本。",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "a_wwps_core_latest" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("🛰️ 正在启动 wwps-core 升级 (最新版本)...")
                        .await?;
                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    tokio::spawn(async move {
                        if let Err(err) = WwpsCoreUpgradeManager::run_upgrade(
                            None,
                            bot_clone.clone(),
                            chat_id_clone,
                        )
                        .await
                        {
                            let _ = bot_clone
                                .send_message(
                                    chat_id_clone,
                                    format!("❌ wwps-core 升级失败: {}", err),
                                )
                                .await;
                        }
                    });
                }
                "a_wwps_core_tags" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("📜 正在获取最近 5 个版本...")
                        .await?;

                    let reply = match WwpsCoreUpgradeConfig::from_env()
                        .and_then(WwpsCoreUpgradeManager::new)
                    {
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
                                bot.edit_message_text(
                                    chat_id,
                                    msg_id,
                                    "请选择要安装的 wwps-core 版本：",
                                )
                                .reply_markup(InlineKeyboardMarkup::new(buttons))
                                .await
                            }
                            Ok(_) => {
                                bot.edit_message_text(
                                    chat_id,
                                    msg_id,
                                    "未获取到可用版本，请稍后重试。",
                                )
                                .await
                            }
                            Err(err) => {
                                bot.edit_message_text(
                                    chat_id,
                                    msg_id,
                                    format!("❌ 获取版本列表失败: {}", err),
                                )
                                .await
                            }
                        },
                        Err(err) => {
                            bot.edit_message_text(
                                chat_id,
                                msg_id,
                                format!("❌ wwps-core 配置错误: {}", err),
                            )
                            .await
                        }
                    };

                    if reply.is_err() {
                        let _ = bot
                            .send_message(
                                chat_id,
                                "❌ 无法获取版本列表，请检查网络或 GitHub 访问。",
                            )
                            .await;
                    }
                }
                d if d.starts_with("wwps_core_tag:") => {
                    let tag = d.strip_prefix("wwps_core_tag:").unwrap_or("").to_string();
                    if tag.is_empty() {
                        bot.answer_callback_query(q.id)
                            .text("❌ 版本信息为空")
                            .await?;
                        return Ok(());
                    }

                    bot.answer_callback_query(q.id.clone())
                        .text(format!("🛰️ 正在升级到版本 {}...", tag))
                        .await?;

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;
                    tokio::spawn(async move {
                        if let Err(err) = WwpsCoreUpgradeManager::run_upgrade(
                            Some(tag),
                            bot_clone.clone(),
                            chat_id_clone,
                        )
                        .await
                        {
                            let _ = bot_clone
                                .send_message(
                                    chat_id_clone,
                                    format!("❌ wwps-core 升级失败: {}", err),
                                )
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

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "📦 <b>Sing-box 管理</b>\n管理 Sing-box 服务状态",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "a_wwps_box_restart" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("🔄 正在重启 Sing-box 服务...")
                        .await?;

                    match SingBoxInstaller::restart_service().await {
                        Ok(_) => {
                            bot.edit_message_text(chat_id, msg_id, "✅ <b>Sing-box 重启成功</b>")
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                        Err(err) => {
                            bot.edit_message_text(chat_id, msg_id, format!("❌ 重启失败: {}", err))
                                .await?;
                        }
                    }
                }
                "a_wwps_box_status" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("📊 正在获取状态...")
                        .await?;

                    match SingBoxInstaller::status().await {
                        Ok(status) => {
                            bot.edit_message_text(
                                chat_id,
                                msg_id,
                                format!("📦 <b>Sing-box 状态</b>\n\n{}", status),
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                        }
                        Err(err) => {
                            bot.edit_message_text(
                                chat_id,
                                msg_id,
                                format!("❌ 获取状态失败: {}", err),
                            )
                            .await?;
                        }
                    }
                }

                "a_tune" => {
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("a_bbr3".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "a_sys_maint" => {
                    if logic::operations::MAINTENANCE_FLAG.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        bot.answer_callback_query(q.id.clone())
                            .text("❌ 配置任务正在执行中，请稍后再试")
                            .await?;
                        return Ok(());
                    }

                    let keyboard =
                        InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                            "⚙️ 配置中... (请等待)",
                            "a_sys_maint_disabled",
                        )]]);
                    let _ = bot
                        .edit_message_reply_markup(chat_id, msg_id)
                        .reply_markup(keyboard)
                        .await;

                    bot.answer_callback_query(q.id.clone())
                        .text("⚙️ 正在配置自动安全更新...")
                        .await?;
                    let bot_c = bot.clone();
                    tokio::spawn(async move {
                        match Operations::perform_maintenance().await {
                            Ok(log) => {
                                let log_tail = if log.len() > 4000 {
                                    format!("... (Truncated)\n{}", &log[log.len() - 3000..])
                                } else {
                                    log
                                };
                                let _ = bot_c
                                    .send_message(
                                        chat_id,
                                        format!(
                                            "✅ <b>自动安全更新配置完成</b>\n\n<pre>{}</pre>",
                                            log_tail
                                        ),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                            Err(e) => {
                                let _ = bot_c
                                    .send_message(
                                        chat_id,
                                        format!("❌ <b>维护失败</b>\n\n原因: {}", e),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }
                    });
                }
                "a_sys_reboot" => {
                    if logic::operations::REBOOT_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
                        bot.answer_callback_query(q.id.clone())
                            .text("❌ 重启任务正在执行中，请稍后再试")
                            .await?;
                        return Ok(());
                    }

                    let keyboard =
                        InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                            "⚠️ 重启中... (请等待)",
                            "a_sys_reboot_disabled",
                        )]]);
                    let _ = bot
                        .edit_message_reply_markup(chat_id, msg_id)
                        .reply_markup(keyboard)
                        .await;

                    bot.answer_callback_query(q.id.clone())
                        .text("⚠️ 系统将于 3 秒后重启...")
                        .await?;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let _ = Operations::reboot_system().await;
                    });
                }
// ==================== Schedule Handler ====================
                d if d.starts_with("a_geo_sched") || d == "geo_sched_off" || d == "m_sched" || d.starts_with("s_add") || d.starts_with("s_custom") || d.starts_with("s_del") || d == "s_add_menu" || d == "s_add_custom_menu" => {
                    match handle_schedule_callback(bot.clone(), chat_id, msg_id, d, q.id.as_str(), state.clone()).await {
                        Ok(CallbackOutcome::Done) => break Ok(()),
                        Ok(CallbackOutcome::Redirect(new_data)) => { data = new_data; continue; }
                        Err(e) => {
                            log::warn!("Schedule handler error: {}", e);
                            break Ok(());
                        }
                    }
                }
                _ => {
                    bot.answer_callback_query(q.id).await?;
                }
            }
            break Ok(());
        }
    })
}

async fn save_config(state: &Arc<AppState>) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);

    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;

    let hash = state.self_destruct_key_hash().await;
    encrypted_config.self_destruct_key_hash = hash;

    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    harden_process();

    // 立即执行防调试检查
    crate::logic::anti_debug::check_debugger();

    // CLI 仅输出模式：先处理，避免 verify_integrity 向 stdout 打印导致安装器把整段输出当成 TOTP 密钥
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "--generate-totp-secret" {
            println!("{}", TotpManager::generate_new_secret());
            return Ok(());
        }
        if args[1] == "-v" || args[1] == "--version" {
            println!("tgbot {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        if args[1] == "--setup" {
            if args.len() < 5 {
                println!("Usage: tgbot --setup <token> <admin_id> <totp_secret>");
                return Ok(());
            }
            return run_setup(&args[2], &args[3], &args[4]).await;
        }
        if args[1] == "--setup-stdin" {
            return run_setup_from_stdin().await;
        }
    }

    // 正常启动：校验完整性后再加载配置
    verify_integrity().await?;

    let config_dir = config_dir();
    let key_path = config_dir.join(KEY_FILE);
    let config_path = config_dir.join(CONFIG_FILE);
    if config_path.exists() && !key_path.exists() {
        anyhow::bail!(
            "配置文件 {} 存在，但 {} 不存在。请将 setup 时生成的 .key 与 config.enc 一并部署到本机，或在本机重新执行 tgbot --setup 完成初始化。",
            config_path.display(),
            key_path.display()
        );
    }
    let security = SecurityManager::new(&key_path).context("Security manager failed")?;
    let config_data = fs::read(&config_path).context("Config file miss")?;
    let encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;

    let token_vec = security
        .decrypt(&encrypted_config.token)
        .context("解密 token 失败")?;
    let admin_id_vec = security
        .decrypt(&encrypted_config.admin_id)
        .context("解密 admin_id 失败")?;
    let totp_sec_vec = security
        .decrypt(&encrypted_config.totp_secret)
        .context("解密 totp_secret 失败")?;

    let token: String = String::from_utf8(token_vec.expose_secret().to_vec())
        .context("token 包含无效的 UTF-8 字符")?;
    let admin_id_str: String = String::from_utf8(admin_id_vec.expose_secret().to_vec())
        .context("admin_id 包含无效的 UTF-8 字符")?;
    let totp_secret: String = String::from_utf8(totp_sec_vec.expose_secret().to_vec())
        .context("totp_secret 包含无效的 UTF-8 字符")?
        .trim()
        .to_string();

    let admin_id: i64 = admin_id_str
        .trim()
        .parse()
        .context("无效的 admin_id 格式 (应为 i64)")?;

    let validator = ConfigValidator::new();
    if let Err(e) = validator.validate_decrypted_config(
        &token,
        admin_id,
        &totp_secret,
        &encrypted_config.self_destruct_key_hash,
    ) {
        anyhow::bail!("❌ 配置校验失败: {}", e);
    }

    let totp_manager_instance = TotpManager::new(&secrecy::SecretString::from(totp_secret.clone()))
        .map_err(|e| anyhow::anyhow!("初始化 TOTP 验证器失败: {}", e))?;

    let bot_settings = BotSettings::load();
    let state = Arc::new(AppState::new(
        admin_id,
        totp_manager_instance,
        production_executor(),
        encrypted_config.self_destruct_key_hash.clone(),
        bot_settings.session_timeout_secs,
    ));

    let bot = Bot::new(&token);
    if let Err(err) = register_bot_commands(&bot).await {
        eprintln!("[WARN] 命令注册失败: {}", err);
    }
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    // 先启动 Dispatcher，再在后台初始化调度器与通知，避免 /start 等命令因启动阻塞而无响应
    println!("🚀 Bot is starting...");
    let bot_for_init = bot.clone();
    tokio::spawn(async move {
        if let Err(e) =
            logic::scheduler::start_scheduler(bot_for_init.clone(), ChatId(admin_id)).await
        {
            log::error!("❌ 初始化调度器失败: {}", e);
        }
        let _ = notify_upgrade_success(&bot_for_init, admin_id).await;
        let _ = notify_bbr3_reboot_result(&bot_for_init, admin_id).await;
        let _ = notify_online(&bot_for_init, admin_id).await;
    });

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tgbot::logic::self_destruct::SelfDestructExecutor;

    struct CountingExecutor {
        calls: Arc<AtomicUsize>,
    }

    impl SelfDestructExecutor for CountingExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn self_destruct_trigger_uses_executor_boundary() {
        let calls = Arc::new(AtomicUsize::new(0));
        let executor = Arc::new(CountingExecutor {
            calls: calls.clone(),
        });

        tgbot::logic::self_destruct::trigger(executor);
        tokio::time::sleep(Duration::from_secs(3)).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn format_duration_human_seconds() {
        assert_eq!(format_duration_human(0), "0秒");
        assert_eq!(format_duration_human(45), "45秒");
    }

    #[test]
    fn format_duration_human_minutes() {
        assert_eq!(format_duration_human(60), "1分钟");
        assert_eq!(format_duration_human(90), "1分钟");
        assert_eq!(format_duration_human(120), "2分钟");
    }

    #[test]
    fn format_duration_human_hours_and_days() {
        assert_eq!(format_duration_human(3600), "1小时");
        assert_eq!(format_duration_human(3661), "1小时1分");
        assert!(format_duration_human(86400).starts_with("1天"));
    }
}

async fn notify_online(bot: &Bot, admin_id: i64) -> Result<()> {
    let ip = match SystemMonitor::get_public_ip().await {
        Ok(ip) => ip,
        Err(err) => {
            log::warn!("获取公网 IPv4 失败: {}", err);
            "Unavailable".to_string()
        }
    };

    // IP Masking: 1.2.3.4 -> 1.2.*.*
    let masked_ip = if ip.contains('.') {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 {
            format!("{}.{}.*.*", parts[0], parts[1])
        } else {
            ip.clone()
        }
    } else {
        ip.clone() // IPv6 or other formatted IP, leave as is or apply specific logic if needed
    };

    // 获取简单的系统信息
    let sys_info = "Linux"; // 可以扩展调用 SystemMonitor 获取更详细信息

    let msg = format!(
        "🤖 **Bot 已上线**\n\n🌍 IP: `{}`\n💻 系统: {}",
        masked_ip, sys_info
    );

    let _ = bot
        .send_message(ChatId(admin_id), msg)
        .parse_mode(ParseMode::MarkdownV2)
        .await;
    Ok(())
}

async fn notify_upgrade_success(bot: &Bot, admin_id: i64) -> Result<()> {
    let flag_path = Path::new(UPGRADE_FLAG_FILE);
    if !flag_path.exists() {
        return Ok(());
    }

    let version_raw = fs::read_to_string(flag_path).unwrap_or_default();
    let version = version_raw.trim();
    if let Err(e) = fs::remove_file(flag_path) {
        eprintln!("[WARN] 无法删除升级标记文件: {}", e);
    }

    let message = if version.is_empty() {
        "✅ Bot 已完成自更新。".to_string()
    } else {
        format!("✅ Bot 已成功更新至 {}。", version)
    };

    bot.send_message(ChatId(admin_id), message).await?;
    Ok(())
}

async fn notify_bbr3_reboot_result(bot: &Bot, admin_id: i64) -> Result<()> {
    let flag_path = Path::new(BBR3_PENDING_FLAG_FILE);
    if !flag_path.exists() {
        return Ok(());
    }

    let info = MaintenanceManager::collect_bbr3_runtime_info().await;

    if let Err(e) = fs::remove_file(flag_path) {
        eprintln!("[WARN] 无法删除 BBR3 标记文件: {}", e);
    }

    let kernel_hint = if info.has_xanmod_kernel { "是" } else { "否" };
    let proc_hint = if info.has_xanmod_proc_version {
        "是"
    } else {
        "否"
    };

    let message = format!(
        "✅ <b>BBR3 重启后校验结果</b>\n\n<code>uname -r</code>\n<code>{}</code>\n\n<code>sysctl net.ipv4.tcp_congestion_control</code>\n<code>net.ipv4.tcp_congestion_control = {}</code>\n\n<code>cat /proc/version</code>\n<code>{}</code>\n\n内核名包含 XanMod: <b>{}</b>\n/proc/version 包含 XanMod: <b>{}</b>",
        info.uname_r, info.tcp_congestion_control, info.proc_version, kernel_hint, proc_hint
    );

    bot.send_message(ChatId(admin_id), message)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}
