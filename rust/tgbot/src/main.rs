// mod logic; // Moved to lib.rs
mod app;
mod bootstrap;

use obfstr::obfstr;

use tgbot::logic;

use crate::app::auth;
use crate::app::destruct_flow::{self, MessageFlowOutcome};
use crate::app::state::{AppState, ScheduleFrequency, ScheduleInputState, TimeoutStatus};
use crate::bootstrap::{
    BOT_VERSION, BotSettings, CONFIG_FILE, ConfigValidator, DEFAULT_SESSION_TIMEOUT_SECS,
    EncryptedConfig, KEY_FILE, config_dir, harden_process, run_setup, run_setup_from_stdin,
    verify_integrity,
};
use tgbot::core::paths::{singbox, xray};
use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode,
};
use teloxide::utils::command::BotCommands;
use tgbot::core::types::IpVersion;
use tgbot::logic::config::{ConfigManager, RealityProto, WarpMode};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::maintenance::{BBR3_PENDING_FLAG_FILE, MaintenanceManager};
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::task_types::TaskType;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::self_destruct::production_executor;
use tgbot::logic::system::SystemMonitor;
use tgbot::logic::totp::TotpManager;
use tgbot::logic::upgrade::{
    UPGRADE_FLAG_FILE, UpgradeManager,
    wwps_core::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager},
};
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

fn format_duration_human(secs: u64) -> String {
    if secs < 60 {
        format!("{}秒", secs)
    } else if secs < 3600 {
        format!("{}分钟", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{}小时", h)
        } else {
            format!("{}小时{}分", h, m)
        }
    } else {
        format!("{}天", secs / 86400)
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn validate_hash_prefix(prefix: &str) -> Result<&str> {
    if prefix.is_empty() {
        anyhow::bail!("hash 前缀不能为空");
    }
    if prefix.len() > 8 {
        anyhow::bail!("hash 前缀过长: {} (最大 8)", prefix.len());
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("hash 前缀包含无效字符");
    }
    Ok(prefix)
}

fn validate_idx(idx: usize, max: usize, field_name: &str) -> Result<()> {
    if idx >= max {
        anyhow::bail!(
            "{} 索引 {} 超出范围 (最大 {})",
            field_name,
            idx,
            max.saturating_sub(1)
        );
    }
    Ok(())
}

const MAX_FILE_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024;

async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context(obfstr!("无法向 Telegram 注册主命令").to_string())?;
    println!("{}", obfstr!("✅ 已向 Telegram 注册主命令"));
    Ok(())
}

async fn show_reality_batch_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    proto: RealityProto,
) -> ResponseResult<()> {
    let (ip_prefix, title) = match proto {
        RealityProto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
        RealityProto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
        RealityProto::XdnsMkcp => ("u_xdns_ip:", "XDNS Finalmask (mKCP+DNS)"),
    };

    // 检测公网 IPv6 是否可用
    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        "🌐 IPv4 (0.0.0.0)",
        format!("{}4", ip_prefix),
    )]];

    // 仅在机器拥有公网 IPv6 时才显示 IPv6 选项
    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("{}6", ip_prefix),
        ));

        // XHTTP 双栈分离选项也依赖 IPv6
        if proto == RealityProto::XHTTP {
            buttons.push(vec![
                InlineKeyboardButton::callback(
                    "🚀 双栈分离 (v6上v4下)",
                    format!("{}s6", ip_prefix),
                ),
                InlineKeyboardButton::callback(
                    "🚀 双栈分离 (v4上v6下)",
                    format!("{}s4", ip_prefix),
                ),
            ]);
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>{} 批量备份 (增强+独立)</b>\n\n✨ <b>自动启用的安全特性:</b>\n• 🎲 随机ShortId (每个配置唯一)\n• 🔄 去重SNI选择 (避免重复)\n• 🏷️ 唯一Tag标识 (基于协议+UUID)\n• 📄 独立配置文件 (不影响原配置)\n\n⬇️ <b>第一步: 请选择网络协议版本:</b>",
            title
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

async fn show_reality_qty_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    ip_version: IpVersion,
    proto: RealityProto,
) -> ResponseResult<()> {
    let ip_ver_code = match ip_version {
        IpVersion::IPv4 => "4",
        IpVersion::IPv6 => "6",
        IpVersion::SplitStackV6Primary => "s6",
        IpVersion::SplitStackV4Primary => "s4",
    };
    let ip_display = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        IpVersion::SplitStackV6Primary => "双栈分离 (v6上v4下)",
        IpVersion::SplitStackV4Primary => "双栈分离 (v4上v6下)",
    };

    let (exec_prefix, title) = match proto {
        RealityProto::Vision => ("u_batch_exec:", "Reality"),
        RealityProto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
        RealityProto::XdnsMkcp => ("u_xdns_exec:", "XDNS"),
    };

    let buttons = vec![
        vec![
            InlineKeyboardButton::callback(
                "1",
                format!("{ip_ver_code}:1")
                    .replace(":1", format!("{}{ip_ver_code}:1", exec_prefix).as_str()),
            ),
            // 上面这种 replace 比较啰嗦，我们直接构造
            InlineKeyboardButton::callback("1", format!("{exec_prefix}{ip_ver_code}:1")),
            InlineKeyboardButton::callback("3", format!("{exec_prefix}{ip_ver_code}:3")),
            InlineKeyboardButton::callback("5", format!("{exec_prefix}{ip_ver_code}:5")),
        ],
        vec![
            InlineKeyboardButton::callback("10", format!("{exec_prefix}{ip_ver_code}:10")),
            InlineKeyboardButton::callback("20", format!("{exec_prefix}{ip_ver_code}:20")),
            InlineKeyboardButton::callback("50", format!("{exec_prefix}{ip_ver_code}:50")),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")], // 统一返回 m_usr
    ];

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>{} 批量备份 (增强+独立)</b>\n\n🌐 网络协议: <b>{}</b>\n\n⬇️ <b>第二步: 请选择生成数量:</b>",
            title, ip_display
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

fn trigger_reality_auto_init(bot: Bot, chat_id: ChatId, msg_id: MessageId) {
    tokio::spawn(async move {
        match RealityInstaller::run(bot.clone(), chat_id, msg_id).await {
            Ok(RealityInstallOutcome::AlreadyReady) => {
                let _ =
                    show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::Vision).await;
            }
            Ok(RealityInstallOutcome::Completed) => {
                let _ =
                    show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::Vision).await;
                let _ = bot
                    .send_message(
                        chat_id,
                        "✅ <b>Reality 母版已初始化完成，可继续批量生成。</b>",
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Ok(RealityInstallOutcome::InProgress) => {
                // 进度信息已在 RealityInstaller 内更新，无需额外处理
            }
            Err(e) => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!(
                            "❌ <b>Reality 环境初始化失败</b>\n原因: {}\n请尝试运维菜单中【初始化 Reality】或手动执行 install.sh 选项 3。",
                            e
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
    });
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
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

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
                save_config(&state)
                    .await
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

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

    if let Some(text) = msg.text() {
        if text.len() > MAX_INPUT_LENGTH {
            bot.send_message(
                chat_id,
                format!("⚠️ 输入过长，请控制在 {} 字符以内。", MAX_INPUT_LENGTH),
            )
            .await?;
            return Ok(());
        }
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
                    .split(|c| c == ',' || c == '，' || c == '\n')
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

fn schedule_task_name(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::SystemMaintenance => "系统维护+重启",
        TaskType::Reboot => "系统重启",
        TaskType::GeoUpdate => "GeoData 更新",
        TaskType::ReloadCore => "重载核心",
    }
}

fn schedule_frequency_name(frequency: &ScheduleFrequency) -> &'static str {
    match frequency {
        ScheduleFrequency::Daily => "每天",
        ScheduleFrequency::Weekly => "每周",
    }
}

fn weekday_label(day: &str) -> &'static str {
    match day {
        "Mon" => "周一",
        "Tue" => "周二",
        "Wed" => "周三",
        "Thu" => "周四",
        "Fri" => "周五",
        "Sat" => "周六",
        "Sun" => "周日",
        _ => "未选择",
    }
}

fn timezone_label(timezone: &str) -> &'static str {
    match timezone {
        "UTC" => "UTC",
        "Asia/Shanghai" => "中国标准时间 (UTC+8)",
        "Asia/Tokyo" => "日本标准时间 (UTC+9)",
        "Asia/Singapore" => "新加坡时间 (UTC+8)",
        "Europe/London" => "英国时间",
        "Europe/Berlin" => "中欧时间",
        "America/New_York" => "美国东部时间",
        "America/Los_Angeles" => "美国太平洋时间",
        _ => "自定义时区",
    }
}

fn build_custom_schedule_text(input: &ScheduleInputState) -> String {
    let task = schedule_task_name(&input.task_type);
    let freq = schedule_frequency_name(&input.frequency);
    let timezone = input.timezone.as_str();
    let timezone_text = timezone_label(timezone);
    let day = input
        .day_of_week
        .as_deref()
        .map(weekday_label)
        .unwrap_or("未选择");
    let hour = input
        .hour
        .map(|h| format!("{:02}", h))
        .unwrap_or_else(|| "--".to_string());
    let minute = input
        .minute
        .map(|m| format!("{:02}", m))
        .unwrap_or_else(|| "--".to_string());

    let day_line = if matches!(input.frequency, ScheduleFrequency::Weekly) {
        format!("\n📅 星期: <b>{}</b>", day)
    } else {
        String::new()
    };

    format!(
        "🧩 <b>自定义定时任务</b>\n\n📌 任务: <b>{}</b>\n🔁 周期: <b>{}</b>{}\n🌍 时区: <b>{}</b>\n   <code>{}</code>\n🕒 时间: <b>{}:{}</b>\n\n请继续点击按钮完成设置。",
        task, freq, day_line, timezone_text, timezone, hour, minute
    )
}

fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📅 选择星期", "s_custom_ui:day"),
            InlineKeyboardButton::callback("🕐 选择小时", "s_custom_ui:hour"),
            InlineKeyboardButton::callback("🕑 选择分钟", "s_custom_ui:minute"),
        ],
        vec![InlineKeyboardButton::callback(
            "🌍 选择时区",
            "s_custom_ui:tz",
        )],
        vec![InlineKeyboardButton::callback(
            "✅ 确认创建任务",
            "s_custom_confirm",
        )],
        vec![InlineKeyboardButton::callback("❌ 取消", "s_custom_cancel")],
        vec![InlineKeyboardButton::callback("⬅️ 返回", return_to)],
    ])
}

fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("周一", "s_custom_set:day:Mon"),
            InlineKeyboardButton::callback("周二", "s_custom_set:day:Tue"),
            InlineKeyboardButton::callback("周三", "s_custom_set:day:Wed"),
            InlineKeyboardButton::callback("周四", "s_custom_set:day:Thu"),
        ],
        vec![
            InlineKeyboardButton::callback("周五", "s_custom_set:day:Fri"),
            InlineKeyboardButton::callback("周六", "s_custom_set:day:Sat"),
            InlineKeyboardButton::callback("周日", "s_custom_set:day:Sun"),
        ],
        vec![InlineKeyboardButton::callback(
            "⬅️ 返回配置",
            "s_custom_ui:main",
        )],
    ])
}

fn build_custom_hour_keyboard() -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in (0u8..24).collect::<Vec<_>>().chunks(6) {
        let row = chunk
            .iter()
            .map(|h| {
                InlineKeyboardButton::callback(
                    format!("{:02}", h),
                    format!("s_custom_set:hour:{:02}", h),
                )
            })
            .collect::<Vec<_>>();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回配置",
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn build_custom_minute_keyboard() -> InlineKeyboardMarkup {
    let minute_points = [0u8, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55];
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in minute_points.chunks(4) {
        let row = chunk
            .iter()
            .map(|m| {
                InlineKeyboardButton::callback(
                    format!("{:02}", m),
                    format!("s_custom_set:minute:{:02}", m),
                )
            })
            .collect::<Vec<_>>();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回配置",
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("UTC", "s_custom_set:tz:UTC"),
            InlineKeyboardButton::callback("中国 (UTC+8)", "s_custom_set:tz:Asia/Shanghai"),
        ],
        vec![
            InlineKeyboardButton::callback("东京 (UTC+9)", "s_custom_set:tz:Asia/Tokyo"),
            InlineKeyboardButton::callback("新加坡 (UTC+8)", "s_custom_set:tz:Asia/Singapore"),
        ],
        vec![
            InlineKeyboardButton::callback("伦敦", "s_custom_set:tz:Europe/London"),
            InlineKeyboardButton::callback("柏林", "s_custom_set:tz:Europe/Berlin"),
        ],
        vec![
            InlineKeyboardButton::callback("纽约", "s_custom_set:tz:America/New_York"),
            InlineKeyboardButton::callback("洛杉矶", "s_custom_set:tz:America/Los_Angeles"),
        ],
        vec![InlineKeyboardButton::callback(
            "⬅️ 返回配置",
            "s_custom_ui:main",
        )],
    ])
}

fn build_cron_from_custom_state(input: &ScheduleInputState) -> Option<String> {
    let hour = input.hour?;
    let minute = input.minute?;
    match input.frequency {
        ScheduleFrequency::Daily => Some(format!("{} {} * * *", minute, hour)),
        ScheduleFrequency::Weekly => input
            .day_of_week
            .as_ref()
            .map(|d| format!("{} {} * * {}", minute, hour, d)),
    }
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
            if is_custom_followup {
                if state
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
                        vec![
                            InlineKeyboardButton::callback("⏰ 定时任务", "m_sched"),
                        ],
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
                        vec![InlineKeyboardButton::callback("🧹 系统维护", "a_sys_maint")],
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
                    let wwps_core_config_exists =
                        Path::new(xray::CONF_DIR).exists();
                    let singbox_config_exists =
                        Path::new(singbox::CONF_DIR).exists();
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
                        buttons.push(vec![InlineKeyboardButton::callback("🅧 Xray-core 管理", "m_xray_mgmt")]);
                        buttons.push(vec![InlineKeyboardButton::callback("📦 Sing-box 管理", "m_singbox_mgmt")]);
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
                "m_xray_mgmt" => {
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    let mut buttons = Vec::new();

                    if inbounds.is_empty() {
                        buttons.push(vec![
                            InlineKeyboardButton::callback(
                                "🚀 Reality 批量备份",
                                "u_batch_init",
                            ),
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
                            InlineKeyboardButton::callback("🚀 XDNS (mKCP+DNS)", "u_xdns_init"),
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
                "m_singbox_mgmt" => {
                    let is_installed = SingBoxInstaller::is_installed().await;
                    let inbounds = SingBoxConfigManager::list_all_inbound_files().await.unwrap_or_default();
                    let mut buttons = Vec::new();

                    if !is_installed {
                        buttons.push(vec![InlineKeyboardButton::callback("🚀 安装 Sing-box", "sb_install")]);
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未检测到 Sing-box</b>\n\n系统尚未安装 Sing-box，无法使用 Hysteria2/TUIC 协议。",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    } else if inbounds.is_empty() {
                        buttons.push(vec![
                            InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                            InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                        ]);
                        buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未找到配置文件</b>\n\n您可以创建 Hysteria2 或 TUIC 批量配置。",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    } else {
                        for (i, path) in inbounds.iter().enumerate() {
                            let filename = path.split('/').next_back().unwrap_or("Unknown");
                            buttons.push(vec![InlineKeyboardButton::callback(
                                format!("📁 {}", filename),
                                format!("sb_l:{}", i),
                            )]);
                        }
                        buttons.push(vec![InlineKeyboardButton::callback("🗑️ 删除管理", "sb_del_cfg")]);
                        buttons.push(vec![
                            InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                            InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                        ]);
                        buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "📦 <b>Sing-box 管理</b>\n选择配置文件:",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    }
                }
                // Sing-box callbacks
                "sb_install" => {
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
                }
                "sb_h2_init" => {
                    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
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
                }
                "sb_tu_init" => {
                    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
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
                }
                d if d.starts_with("sb_h2_obfs:") => {
                    let parts: Vec<&str> = d.strip_prefix("sb_h2_obfs:").unwrap_or("").split(':').collect();
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
                }
                d if d.starts_with("sb_h2_exec:") => {
                    let parts: Vec<&str> = d.strip_prefix("sb_h2_exec:").unwrap_or("").split(':').collect();
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

                                // 发送结果信息
                                let result_msg = format!(
                                    "✅ 批量创建完成！\n\n📊 生成数量: {}",
                                    result.created_count
                                );
                                if let Ok(msg) = bot_clone.send_message(chat_id_clone, result_msg).await {
                                    message_ids.push(msg.id);
                                }

                                // 启动后台任务，60秒后自动删除所有消息
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
                }
                d if d.starts_with("sb_tu_exec:") => {
                    let parts: Vec<&str> = d.strip_prefix("sb_tu_exec:").unwrap_or("").split(':').collect();
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

                                // 发送结果信息
                                let result_msg = format!(
                                    "✅ 批量创建完成！\n\n📊 生成数量: {}",
                                    result.created_count
                                );
                                if let Ok(msg) = bot_clone.send_message(chat_id_clone, result_msg).await {
                                    message_ids.push(msg.id);
                                }

                                // 启动后台任务，60秒后自动删除所有消息
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
                "m_log" => {
                    let has_access = Path::new(xray::ACCESS_LOG).exists();
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::callback(
                                if has_access {
                                    "🔴 关闭 Access 日志"
                                } else {
                                    "🟢 开启 Access 日志"
                                },
                                "l_tgl",
                            ),
                            InlineKeyboardButton::callback("📝 查看 Access 日志", "l_tail_acc"),
                        ],
                        vec![
                            InlineKeyboardButton::callback("📝 查看 Error 日志", "l_tail_err"),
                            InlineKeyboardButton::callback("🔄 刷新日志", "m_log"),
                        ],
                        vec![InlineKeyboardButton::callback(
                            "⬅️ 返回运维中心",
                            "m_ops_center",
                        )],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!(
                            "📄 <b>日志管理</b>\nAccess 日志状态: {}",
                            if has_access {
                                "🟢 已开启"
                            } else {
                                "🔴 已关闭"
                            }
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "m_session_timeout" => {
                    let current = state.session_timeout_secs().await;
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

                    bot.edit_message_text(
                    chat_id,
                    msg_id,
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
                    state.set_session_timeout_secs(secs).await;
                    let settings = BotSettings {
                        session_timeout_secs: secs,
                    };
                    if let Err(e) = settings.save() {
                        log::error!("保存会话设置失败: {}", e);
                    }
                    bot.answer_callback_query(q.id.clone())
                        .text(format!(
                            "✅ 会话有效期已设为 {}",
                            format_duration_human(secs)
                        ))
                        .await?;

                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("m_session_timeout".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "m_danger" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "💥 立即自毁 (VPS过期一键删)",
                            "a_destroy_ask",
                        )],
                        vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "⚠️ <b>危险区域</b>\n\n此处包含不可逆的破坏性操作。\n请谨慎操作！",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }

                // ... Skipping a_warp_switch_mode and a_del_warp updates for brevity in this chunk if tool allows multiple replacements via array OR I will make separate calls.
                // The tool allows LIST of chunks. I will provide multiple chunks.
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
                }
                "a_warp_switch_mode" => {
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
                            continue;
                        }
                        Err(e) => {
                            bot.answer_callback_query(q.id)
                                .text(format!("❌ 切换失败: {}", e))
                                .await?;
                        }
                    }
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
                "a_inst_warp" => {
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
                            continue;
                        }
                        Err(e) => {
                            bot.send_message(chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e))
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                    }
                }
                "a_warp_add_input" => {
                    state.start_warp_input(chat_id, Instant::now()).await;
                    bot.send_message(
                    chat_id,
                    "✏️ <b>请输入要添加的分流规则</b>\n\n支持格式: `geosite:google, domain:reddit.com`\n多个规则请用逗号或换行分隔。\n\n(输入将在 60 秒后超时)",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                }
                "a_warp_del_menu" => {
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
                }
                d if d.starts_with("a_warp_del:") => {
                    let hash_prefix = d.strip_prefix("a_warp_del:").unwrap_or("");
                    if let Err(e) = validate_hash_prefix(hash_prefix) {
                        bot.answer_callback_query(q.id.clone())
                            .text(&format!("❌ {}", e))
                            .await?;
                        continue;
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
                        // Show confirmation
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
                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some("a_warp_del_menu".to_string()),
                            ..new_q
                        };
                        continue;
                    }
                }
                d if d.starts_with("a_warp_del_confirm:") => {
                    let hash_prefix = d.strip_prefix("a_warp_del_confirm:").unwrap_or("");
                    if let Err(e) = validate_hash_prefix(hash_prefix) {
                        bot.answer_callback_query(q.id.clone())
                            .text(&format!("❌ {}", e))
                            .await?;
                        continue;
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
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("a_warp_del_menu".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "a_warp_clear_confirm" => {
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
                }
                "a_warp_clear_exec" => {
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
                            continue;
                        }
                        Err(e) => {
                            bot.answer_callback_query(q.id)
                                .text(format!("❌ 清空失败: {}", e))
                                .await?;
                        }
                    }
                }
                "a_warp_status" => match WarpInstaller::status().await {
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
                },
                "a_bbr3" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("🚀 正在启动 BBR3 安装...")
                        .await?;

                    let bot_clone = bot.clone();
                    let chat_id_clone = chat_id;

                    tokio::spawn(async move {
                        let total_steps = 8u8;

                        let send_result = bot_clone
                        .send_message(
                            chat_id_clone,
                            "🚀 <b>BBR3 + 通用优化安装</b>\n\n⬛⬛⬛⬛⬛⬛⬛⬛⬛⬛ 0%\n\n步骤 1/8: 准备中...",
                        )
                        .parse_mode(ParseMode::Html)
                        .await;

                        if let Err(e) = send_result {
                            eprintln!("[ERROR] 发送初始消息失败: {}", e);
                            let _ = bot_clone
                                .send_message(chat_id_clone, format!("❌ 启动失败: {}", e))
                                .await;
                            return;
                        }

                        let init_msg = send_result.expect("发送初始消息失败");
                        let msg_id = init_msg.id;

                        let step_labels = [
                            "🔧 修复主机名解析...",
                            "📦 安装依赖...",
                            "🔍 检测 CPU 级别...",
                            "⬇️ 添加 XanMod GPG...",
                            "📦 添加 APT 源...",
                            "🔄 更新软件包列表...",
                            "📥 安装 XanMod 内核...",
                            "⚙️ 应用网络优化...",
                        ];

                        let bot_for_progress = bot_clone.clone();
                        let chat_for_progress = chat_id_clone;
                        let msg_id_for_progress = msg_id;

                        let result = MaintenanceManager::install_bbr3_with_progress(
                            move |step: u8, desc: &str| {
                                let filled = "🟩".repeat(step as usize);
                                let empty = "⬛".repeat((total_steps - step) as usize);
                                let percent = (step as f64 / total_steps as f64 * 100.0) as u32;

                                let step_label = if ((step - 1) as usize) < step_labels.len() {
                                    step_labels[(step - 1) as usize]
                                } else {
                                    desc
                                };

                                let text = format!(
                                    "🚀 <b>BBR3 + 通用优化安装</b>\n\n{}{} {}%\n\n步骤 {}/{}: {}",
                                    filled, empty, percent, step, total_steps, step_label
                                );

                                let bot = bot_for_progress.clone();
                                let chat = chat_for_progress;
                                let msg = msg_id_for_progress;
                                tokio::spawn(async move {
                                    let _ = bot
                                        .edit_message_text(chat, msg, text)
                                        .parse_mode(ParseMode::Html)
                                        .await;
                                });
                            },
                        )
                        .await;

                        match result {
                            Ok(result) => {
                                let reboot_notice = if result.reboot_required {
                                    "需要重启系统后切换到新内核并生效。"
                                } else {
                                    "当前无需重启。"
                                };
                                let reply_markup = if result.reboot_required {
                                    InlineKeyboardMarkup::new(vec![
                                        vec![
                                            InlineKeyboardButton::callback(
                                                "🔄 立即重启",
                                                "a_bbr3_reboot_now",
                                            ),
                                            InlineKeyboardButton::callback(
                                                "🕒 稍后重启",
                                                "a_bbr3_reboot_later",
                                            ),
                                        ],
                                        vec![InlineKeyboardButton::callback(
                                            "⬅️ 返回网络优化",
                                            "m_net_opt",
                                        )],
                                    ])
                                } else {
                                    InlineKeyboardMarkup::new(vec![vec![
                                        InlineKeyboardButton::callback(
                                            "⬅️ 返回网络优化",
                                            "m_net_opt",
                                        ),
                                    ]])
                                };
                                let _ = bot_clone
                                .edit_message_text(
                                    chat_id_clone,
                                    msg_id,
                                    format!(
                                        "✅ <b>BBR3 + 通用优化流程已完成</b>\n\n当前内核: <code>{}</code>\n当前拥塞控制算法: <code>{}</code>\n\n已写入合并后的网络优化参数。\n\n<b>注意:</b> {}",
                                        result.kernel_version, result.congestion_control, reboot_notice
                                    ),
                                )
                                .parse_mode(ParseMode::Html)
                                .reply_markup(reply_markup)
                                .await;
                            }
                            Err(e) => {
                                let _ = bot_clone
                                    .edit_message_text(
                                        chat_id_clone,
                                        msg_id,
                                        format!("❌ <b>BBR3 + 通用优化失败</b>\n原因: {}", e),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }
                    });
                }
                "a_bbr3_reboot_now" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("⚠️ 系统将于 3 秒后重启...")
                        .await?;
                    bot.send_message(
                        chat_id,
                        "⚠️ <b>系统将于 3 秒后重启</b>\nBBR3 新内核将在重启后生效。",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let _ = Operations::reboot_system().await;
                    });
                }
                "a_bbr3_reboot_later" => {
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 已选择稍后重启")
                        .await?;
                    bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "✅ <b>已记录为稍后重启</b>\n\nBBR3 已安装完成，待你手动重启系统后切换到新内核生效。",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("⬅️ 返回网络优化", "m_net_opt"),
                ]]))
                .await?;
                }
                "a_warp_restart" => {
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
                }
                "a_warp_uninstall" => {
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
                    "⚠️ <b>卸载确认</b>\n\n确定要卸载 Cloudflare WARP 吗？\n这将移除所有相关组件和配置。"
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
                }
                "a_warp_uninstall_confirm" => {
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
                            continue;
                        }
                        Err(e) => {
                            bot.send_message(chat_id, format!("❌ <b>卸载失败</b>\n原因: {}", e))
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                    }
                }
                "u_batch_init" => {
                    if MaintenanceManager::is_reality_base_ready().await {
                        show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::Vision)
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
                        show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::XHTTP)
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
                "u_xdns_init" => {
                    if !MaintenanceManager::is_reality_base_ready().await {
                        bot.answer_callback_query(q.id.clone())
                            .text("⏳ 正在准备基础环境，请稍候...")
                            .await?;
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "⏳ <b>正在自动初始化基础环境...</b>\n请稍候，完成后会自动进入 XDNS 配置界面。",
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                        trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                        return Ok(());
                    }

                    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
                    let mut buttons = vec![vec![InlineKeyboardButton::callback(
                        "🌐 IPv4 (0.0.0.0)",
                        "u_xdns_ip:4",
                    )]];

                    if has_ipv6 {
                        buttons[0].push(InlineKeyboardButton::callback(
                            "🌐 IPv6 (::)",
                            "u_xdns_ip:6",
                        ));
                    }

                    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")]);

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🚀 <b>XDNS Finalmask 批量配置</b>\n\n✨ <b>特点:</b>\n• DNS 查询流量伪装\n• 适合仅允许 DNS 的受限网络\n• mKCP 可靠传输 (MTU=130)\n\n⬇️ <b>请选择网络协议版本:</b>",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
                d if d.starts_with("u_xdns_ip:") => {
                    let ip_ver_code = d.strip_prefix("u_xdns_ip:").unwrap_or("4");
                    let ip_version = match ip_ver_code {
                        "6" => IpVersion::IPv6,
                        _ => IpVersion::IPv4,
                    };

                    let ip_display = match ip_version {
                        IpVersion::IPv4 => "IPv4",
                        IpVersion::IPv6 => "IPv6",
                        _ => "IPv4",
                    };

                    let buttons = vec![
                        vec![
                            InlineKeyboardButton::callback(
                                "1",
                                format!("u_xdns_exec:{}:1", ip_ver_code),
                            ),
                            InlineKeyboardButton::callback(
                                "3",
                                format!("u_xdns_exec:{}:3", ip_ver_code),
                            ),
                            InlineKeyboardButton::callback(
                                "5",
                                format!("u_xdns_exec:{}:5", ip_ver_code),
                            ),
                        ],
                        vec![
                            InlineKeyboardButton::callback(
                                "10",
                                format!("u_xdns_exec:{}:10", ip_ver_code),
                            ),
                            InlineKeyboardButton::callback(
                                "20",
                                format!("u_xdns_exec:{}:20", ip_ver_code),
                            ),
                            InlineKeyboardButton::callback(
                                "50",
                                format!("u_xdns_exec:{}:50", ip_ver_code),
                            ),
                        ],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "u_xdns_init")],
                    ];

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!(
                            "🚀 <b>XDNS Finalmask 批量配置</b>\n\n🌐 网络协议: <b>{}</b>\n\n⬇️ <b>请选择生成数量:</b>",
                            ip_display
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
                d if d.starts_with("u_xdns_exec:") => {
                    let parts: Vec<&str> = d
                        .strip_prefix("u_xdns_exec:")
                        .unwrap_or("")
                        .split(':')
                        .collect();
                    if parts.len() != 2 {
                        return Ok(());
                    }

                    let ip_ver_code = parts[0];
                    let n: usize = parts[1].parse().unwrap_or(0);

                    let ip_version = match ip_ver_code {
                        "6" => IpVersion::IPv6,
                        _ => IpVersion::IPv4,
                    };

                    let ip_str = match ip_version {
                        IpVersion::IPv4 => "IPv4",
                        IpVersion::IPv6 => "IPv6",
                        _ => "IPv4",
                    };

                    bot.answer_callback_query(q.id.clone())
                        .text(format!("⏳ 正在生成 {} 个 XDNS 配置...", n))
                        .await?;

                    let res = ConfigManager::batch_create_xdns_mkcp(n, true, ip_version).await;

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
                            let temp_file_path = format!("/tmp/wwps_xdns_links_{}.txt", timestamp);

                            if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                                log::warn!("写入临时文件失败: {}", e);
                            } else {
                                let document_sent = bot
                                    .send_document(chat_id, InputFile::file(&temp_file_path))
                                    .caption("XDNS Finalmask 完整链接列表")
                                    .await;

                                if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                                    log::warn!("删除临时文件失败: {}", e);
                                }

                                if let Ok(msg) = document_sent {
                                    message_ids.push(msg.id);
                                }
                            }

                            let mut result_msg = format!(
                                "✅ XDNS Finalmask 批量生成完成！\n\n📊 生成数量: {}\n🌐 网络协议: {}\n⚡ 特点: DNS伪装 + mKCP传输 (MTU=130)",
                                result.created_count, ip_str
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
                d if d.starts_with("u_batch_ip_init:")
                    || d.starts_with("u_xhttp_batch_ip_init:") =>
                {
                    let (prefix, proto) = if d.starts_with("u_batch_ip_init:") {
                        ("u_batch_ip_init:", RealityProto::Vision)
                    } else {
                        ("u_xhttp_batch_ip_init:", RealityProto::XHTTP)
                    };
                    let ip_ver_code = d.strip_prefix(prefix).unwrap_or("");
                    let ip_version = match ip_ver_code {
                        "6" => IpVersion::IPv6,
                        "s6" => IpVersion::SplitStackV6Primary,
                        "s4" => IpVersion::SplitStackV4Primary,
                        _ => IpVersion::IPv4,
                    };
                    // 进入第二步：选择数量
                    show_reality_qty_prompt(&bot, chat_id, msg_id, ip_version, proto).await?;
                }
                d if d.starts_with("u_batch_exec:") || d.starts_with("u_xhttp_batch_exec:") => {
                    let (prefix, proto) = if d.starts_with("u_batch_exec:") {
                        ("u_batch_exec:", RealityProto::Vision)
                    } else {
                        ("u_xhttp_batch_exec:", RealityProto::XHTTP)
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

                    let standalone_mode = true;
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
                        RealityProto::Vision => "Reality",
                        RealityProto::XHTTP => "XHTTP",
                        RealityProto::XdnsMkcp => "XDNS",
                    };

                    bot.answer_callback_query(q.id.clone())
                        .text(format!(
                            "⏳ 正在生成 {} 个 {} 增强配置 ({}, 独立文件)...",
                            n, proto_str, ip_str
                        ))
                        .await?;

                    let res = match proto {
                        RealityProto::Vision => {
                            ConfigManager::batch_create_reality_vision_enhanced(
                                n,
                                standalone_mode,
                                ip_version,
                            )
                            .await
                        }
                        RealityProto::XHTTP => {
                            ConfigManager::batch_create_xhttp_reality_enhanced(
                                n,
                                standalone_mode,
                                ip_version,
                            )
                            .await
                        }
                        RealityProto::XdnsMkcp => {
                            ConfigManager::batch_create_xdns_mkcp(n, standalone_mode, ip_version)
                                .await
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
                            if !combined_links.is_empty() {
                                if let Ok(msg) = bot
                                    .send_message(chat_id, combined_links)
                                    .parse_mode(ParseMode::Html)
                                    .await
                                {
                                    message_ids.push(msg.id);
                                }
                            }

                            // 生成 .txt 附件文件
                            let links_text = result.links.join("\n");
                            let timestamp = chrono::Utc::now().timestamp();
                            let temp_file_path =
                                format!("/tmp/wwps_reality_links_{}.txt", timestamp);

                            // 写入临时文件
                            if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                                log::warn!("写入临时文件失败: {}", e);
                            } else {
                                // 发送文档
                                let document_sent = bot
                                    .send_document(chat_id, InputFile::file(&temp_file_path))
                                    .caption("完整链接列表，建议尽快复制/导入")
                                    .await;

                                // 无论发送成功或失败，都立即删除临时文件
                                if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                                    log::warn!("删除临时文件失败: {}", e);
                                }

                                // 如果发送成功，收集文档消息的 ID
                                if let Ok(msg) = document_sent {
                                    message_ids.push(msg.id);
                                }
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
                            let chat_id_clone = chat_id.clone();
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
                                trigger_reality_auto_init(bot.clone(), chat_id, msg_id);
                            } else {
                                bot.send_message(chat_id, format!("❌ 生成失败: {}", err_msg))
                                    .await?;
                            }
                        }
                    }
                }
                // 用户列表
                d if d.starts_with("u_l:") => {
                    let idx: usize = d.strip_prefix("u_l:").unwrap_or("0").parse().unwrap_or(0);
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    if let Err(e) = validate_idx(idx, inbounds.len(), "用户配置") {
                        bot.answer_callback_query(q.id.clone())
                            .text(&format!("❌ {}", e))
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
                            format!("⚠️ <b>删除确认</b>\n\n您确定要删除用户 <code>{}</code> 吗？\n(注意：当前版本暂未实现单个用户删除逻辑，此操作可能仅用于演示 UI)", escape_html(email))
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
                "m_del_cfg" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "🧨 删除全部配置",
                            "cfg_del_all_confirm",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "➗ 按数量删除配置",
                            "cfg_del_count",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "🎯 指定配置删除",
                            "cfg_del_select",
                        )],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🗑️ <b>删除管理</b>\n请选择删除方式 (操作不可逆):",
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
                "cfg_del_all_confirm" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "⚠️ 确认清空所有配置 (不可恢复) ⚠️",
                            "cfg_del_all_exec",
                        )],
                        vec![InlineKeyboardButton::callback("⬅️ 取消", "m_del_cfg")],
                    ]);
                    bot.edit_message_text(chat_id, msg_id, "🚨 <b>二次确认</b>\n您确定要删除 <b>所有</b> 动态入站配置文件吗？\n此操作将清空所有 batch_* 文件并重启核心。")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                // 执行删除所有配置
                "cfg_del_all_exec" => {
                    let count = ConfigManager::delete_all_configurations()
                        .await
                        .unwrap_or(0);
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
                "cfg_del_count" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::callback("10 个", "cfg_del_exec_count:10"),
                            InlineKeyboardButton::callback("50 个", "cfg_del_exec_count:50"),
                        ],
                        vec![
                            InlineKeyboardButton::callback("100 个", "cfg_del_exec_count:100"),
                            InlineKeyboardButton::callback("500 个", "cfg_del_exec_count:500"),
                        ],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_del_cfg")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "➗ <b>按数量删除 (由旧到新)</b>\n请选择要删除的文件数量:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                // 执行按数量删除
                d if d.starts_with("cfg_del_exec_count:") => {
                    let n: usize = d
                        .strip_prefix("cfg_del_exec_count:")
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    let deleted = ConfigManager::delete_configurations_by_count(n)
                        .await
                        .unwrap_or(0);
                    bot.answer_callback_query(q.id.clone())
                        .text(format!("✅ 已成功清理 {} 个旧配置", deleted))
                        .show_alert(true)
                        .await?;
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some("m_del_cfg".to_string()),
                        ..new_q
                    };
                    continue;
                }
                "cfg_del_select" => {
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    let mut buttons = Vec::new();
                    for (i, path) in inbounds.iter().enumerate().take(50) {
                        // 最多显示50个
                        let filename = path.split('/').next_back().unwrap_or("Unknown");
                        buttons.push(vec![InlineKeyboardButton::callback(
                            format!("🗑 {}", filename),
                            format!("cfg_del_file:{}", i),
                        )]);
                    }
                    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_del_cfg")]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🎯 <b>指定配置删除</b>\n点击以永久删除对应文件:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
                // 确认删除配置
                d if d.starts_with("cfg_del_file:") => {
                    let idx: usize = d
                        .strip_prefix("cfg_del_file:")
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();

                    if let Some(path) = inbounds.get(idx) {
                        let filename = path.split('/').next_back().unwrap_or("Unknown");

                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback(
                                "⚠️ 确认删除",
                                format!("cfg_del_confirm:{}", idx),
                            )],
                            vec![InlineKeyboardButton::callback("🔙 取消", "cfg_del_select")],
                        ]);

                        bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!("⚠️ <b>删除确认</b>\n\n您确定要删除配置文件 <code>{}</code> 吗？\n此操作不可恢复！", escape_html(filename))
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
                // 执行配置删除
                d if d.starts_with("cfg_del_confirm:") => {
                    let idx: usize = d
                        .strip_prefix("cfg_del_confirm:")
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();

                    if let Err(e) = validate_idx(idx, inbounds.len(), "配置文件") {
                        bot.answer_callback_query(q.id.clone())
                            .text(&format!("❌ {}", e))
                            .await?;
                        continue;
                    }

                    if let Some(path) = inbounds.get(idx) {
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
                        data: Some("cfg_del_select".to_string()),
                        ..new_q
                    };
                    continue;
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
                            bot.edit_message_text(
                                chat_id,
                                msg_id,
                                "✅ <b>Sing-box 重启成功</b>",
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                        }
                        Err(err) => {
                            bot.edit_message_text(
                                chat_id,
                                msg_id,
                                format!("❌ 重启失败: {}", err),
                            )
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
                            .text("❌ 维护任务正在执行中，请稍后再试")
                            .await?;
                        return Ok(());
                    }

                    let keyboard =
                        InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                            "🔄 维护中... (请等待)",
                            "a_sys_maint_disabled",
                        )]]);
                    let _ = bot
                        .edit_message_reply_markup(chat_id, msg_id)
                        .reply_markup(keyboard)
                        .await;

                    bot.answer_callback_query(q.id.clone())
                        .text("🧹 正在执行系统维护...")
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
                                            "✅ <b>系统维护完成</b>\n\n<pre>{}</pre>",
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
                "m_sched" => {
                    state.remove_schedule_input(chat_id).await;
                    let summary = if let Some(manager) = logic::scheduler::get_manager().await {
                        manager.get_summary().await
                    } else {
                        "❌ 调度器未初始化".to_string()
                    };

                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::callback("➕ 添加任务", "s_add_menu"),
                            InlineKeyboardButton::callback("➖ 删除任务", "s_del_menu"),
                        ],
                        vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
                    ]);

                    bot.edit_message_text(chat_id, msg_id, summary)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                }
                "s_add_menu" => {
                    state.remove_schedule_input(chat_id).await;
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "每周日 4点 维护+重启",
                            "s_add:maint_sun_4",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "每天 4点 重启核心",
                            "s_add:reload_daily_4",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "🕒 自定义每天/每周时间",
                            "s_add_custom_menu",
                        )],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "➕ <b>添加快速任务</b>\n请选择预设模板:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                "s_add_custom_menu" => {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::callback(
                                "维护+重启 - 每天",
                                "s_custom:maint:daily",
                            ),
                            InlineKeyboardButton::callback(
                                "维护+重启 - 每周",
                                "s_custom:maint:weekly",
                            ),
                        ],
                        vec![
                            InlineKeyboardButton::callback("Geo更新 - 每天", "s_custom:geo:daily"),
                            InlineKeyboardButton::callback("Geo更新 - 每周", "s_custom:geo:weekly"),
                        ],
                        vec![
                            InlineKeyboardButton::callback(
                                "重载核心 - 每天",
                                "s_custom:reload:daily",
                            ),
                            InlineKeyboardButton::callback(
                                "重载核心 - 每周",
                                "s_custom:reload:weekly",
                            ),
                        ],
                        vec![
                            InlineKeyboardButton::callback(
                                "系统重启 - 每天",
                                "s_custom:reboot:daily",
                            ),
                            InlineKeyboardButton::callback(
                                "系统重启 - 每周",
                                "s_custom:reboot:weekly",
                            ),
                        ],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "s_add_menu")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "🧩 <b>自定义定时任务</b>\n先选择任务类型和周期:",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
                d if d.starts_with("s_custom:") => {
                    let mut parts = d.split(':');
                    let _prefix = parts.next();
                    let task_part = parts.next();
                    let freq_part = parts.next();

                    let (task_type, frequency) = match (task_part, freq_part) {
                        (Some("maint"), Some("daily")) => {
                            (TaskType::SystemMaintenance, ScheduleFrequency::Daily)
                        }
                        (Some("maint"), Some("weekly")) => {
                            (TaskType::SystemMaintenance, ScheduleFrequency::Weekly)
                        }
                        (Some("geo"), Some("daily")) => {
                            (TaskType::GeoUpdate, ScheduleFrequency::Daily)
                        }
                        (Some("geo"), Some("weekly")) => {
                            (TaskType::GeoUpdate, ScheduleFrequency::Weekly)
                        }
                        (Some("reload"), Some("daily")) => {
                            (TaskType::ReloadCore, ScheduleFrequency::Daily)
                        }
                        (Some("reload"), Some("weekly")) => {
                            (TaskType::ReloadCore, ScheduleFrequency::Weekly)
                        }
                        (Some("reboot"), Some("daily")) => {
                            (TaskType::Reboot, ScheduleFrequency::Daily)
                        }
                        (Some("reboot"), Some("weekly")) => {
                            (TaskType::Reboot, ScheduleFrequency::Weekly)
                        }
                        _ => {
                            bot.answer_callback_query(q.id)
                                .text("❌ 无效的自定义任务模板")
                                .await?;
                            return Ok(());
                        }
                    };

                    let return_to = match &task_type {
                        TaskType::GeoUpdate => "a_geo_sched_menu",
                        _ => "s_add_custom_menu",
                    };
                    state
                        .insert_schedule_input(
                            chat_id,
                            ScheduleInputState {
                                updated_at: Instant::now(),
                                task_type: task_type.clone(),
                                frequency: frequency.clone(),
                                timezone: "UTC".to_string(),
                                day_of_week: None,
                                hour: None,
                                minute: None,
                                return_to: return_to.to_string(),
                            },
                        )
                        .await;

                    let Some(input_state) = state.schedule_input_snapshot(chat_id).await else {
                        return Ok(());
                    };
                    let text = build_custom_schedule_text(&input_state);
                    let ret = input_state.return_to.clone();

                    bot.edit_message_text(chat_id, msg_id, text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(build_custom_schedule_keyboard(&ret))
                        .await?;
                }
                "s_custom_ui:main" => {
                    if let Some((text, ret)) = state
                        .with_schedule_input(chat_id, |input| {
                            input.updated_at = Instant::now();
                            (build_custom_schedule_text(input), input.return_to.clone())
                        })
                        .await
                    {
                        bot.edit_message_text(chat_id, msg_id, text)
                            .parse_mode(ParseMode::Html)
                            .reply_markup(build_custom_schedule_keyboard(&ret))
                            .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                "s_custom_ui:day" => {
                    if let Some(is_daily) = state
                        .with_schedule_input(chat_id, |input| {
                            input.updated_at = Instant::now();
                            matches!(input.frequency, ScheduleFrequency::Daily)
                        })
                        .await
                    {
                        if is_daily {
                            bot.answer_callback_query(q.id)
                                .text("ℹ️ 每天任务无需选择星期")
                                .await?;
                        } else {
                            let text = "📅 <b>选择每周执行的星期</b>";
                            bot.edit_message_text(chat_id, msg_id, text)
                                .parse_mode(ParseMode::Html)
                                .reply_markup(build_custom_day_keyboard())
                                .await?;
                        }
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                "s_custom_ui:hour" => {
                    if state
                        .with_schedule_input(chat_id, |input| input.updated_at = Instant::now())
                        .await
                        .is_some()
                    {
                        bot.edit_message_text(chat_id, msg_id, "🕐 <b>选择执行小时</b>")
                            .parse_mode(ParseMode::Html)
                            .reply_markup(build_custom_hour_keyboard())
                            .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                "s_custom_ui:minute" => {
                    if state
                        .with_schedule_input(chat_id, |input| input.updated_at = Instant::now())
                        .await
                        .is_some()
                    {
                        bot.edit_message_text(chat_id, msg_id, "🕑 <b>选择执行分钟</b>")
                            .parse_mode(ParseMode::Html)
                            .reply_markup(build_custom_minute_keyboard())
                            .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                "s_custom_ui:tz" => {
                    if state
                        .with_schedule_input(chat_id, |input| input.updated_at = Instant::now())
                        .await
                        .is_some()
                    {
                        bot.edit_message_text(chat_id, msg_id, "🌍 <b>选择任务时区</b>")
                            .parse_mode(ParseMode::Html)
                            .reply_markup(build_custom_timezone_keyboard())
                            .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                d if d.starts_with("s_custom_set:") => {
                    let mut parts = d.split(':');
                    let _ = parts.next(); // s_custom_set
                    let field = parts.next();
                    let value = parts.next();

                    if let Some((text, ret)) = state
                        .with_schedule_input(chat_id, |input| {
                            input.updated_at = Instant::now();
                            match (field, value) {
                                (
                                    Some("day"),
                                    Some(
                                        v @ ("Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun"),
                                    ),
                                ) => {
                                    input.day_of_week = Some(v.to_string());
                                }
                                (Some("hour"), Some(v)) => {
                                    if let Ok(hour) = v.parse::<u8>()
                                        && hour <= 23
                                    {
                                        input.hour = Some(hour);
                                    }
                                }
                                (Some("minute"), Some(v)) => {
                                    if let Ok(minute) = v.parse::<u8>()
                                        && minute <= 59
                                    {
                                        input.minute = Some(minute);
                                    }
                                }
                                (
                                    Some("tz"),
                                    Some(
                                        v @ ("UTC"
                                        | "Asia/Shanghai"
                                        | "Asia/Tokyo"
                                        | "Asia/Singapore"
                                        | "Europe/London"
                                        | "Europe/Berlin"
                                        | "America/New_York"
                                        | "America/Los_Angeles"),
                                    ),
                                ) => {
                                    input.timezone = v.to_string();
                                }
                                _ => {}
                            }
                            (build_custom_schedule_text(input), input.return_to.clone())
                        })
                        .await
                    {
                        bot.edit_message_text(chat_id, msg_id, text)
                            .parse_mode(ParseMode::Html)
                            .reply_markup(build_custom_schedule_keyboard(&ret))
                            .await?;
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                    }
                }
                "s_custom_confirm" => {
                    let Some((cron, task_type, timezone, return_to)) = state
                        .with_schedule_input(chat_id, |input| {
                            input.updated_at = Instant::now();
                            (
                                build_cron_from_custom_state(input),
                                input.task_type.clone(),
                                input.timezone.clone(),
                                input.return_to.clone(),
                            )
                        })
                        .await
                    else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 自定义定时会话不存在，请重新进入。")
                            .await?;
                        return Ok(());
                    };

                    let Some(cron_expression) = cron else {
                        bot.answer_callback_query(q.id)
                            .text("⚠️ 配置不完整，请先选择必要时间项。")
                            .show_alert(true)
                            .await?;
                        return Ok(());
                    };

                    state.remove_schedule_input(chat_id).await;
                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let task = logic::scheduler::ScheduledTask::new_with_timezone(
                            task_type,
                            &cron_expression,
                            &timezone,
                        );
                        let result = manager
                            .add_new_task(bot.clone(), state.admin_id(), task)
                            .await;
                        match result {
                            Ok(_) => {
                                bot.answer_callback_query(q.id)
                                    .text("✅ 任务添加成功")
                                    .await?;
                                let back_label = if return_to == "a_geo_sched_menu" {
                                    "⬅️ 返回 Geo 调度"
                                } else {
                                    "⬅️ 返回定时任务"
                                };
                                bot.edit_message_text(
                                    chat_id,
                                    msg_id,
                                    format!(
                                        "✅ 任务已创建\nCron: <code>{}</code>\nTZ: <code>{}</code>",
                                        cron_expression, timezone
                                    ),
                                )
                                .parse_mode(ParseMode::Html)
                                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                                    InlineKeyboardButton::callback(back_label, &return_to),
                                ]]))
                                .await?;
                            }
                            Err(e) => {
                                bot.answer_callback_query(q.id)
                                    .text("❌ 添加任务失败")
                                    .show_alert(true)
                                    .await?;
                                bot.edit_message_text(
                                    chat_id,
                                    msg_id,
                                    format!("❌ 添加任务失败: {}", e),
                                )
                                .await?;
                            }
                        }
                    } else {
                        bot.answer_callback_query(q.id)
                            .text("❌ 调度器未初始化")
                            .await?;
                    }
                }
                "s_custom_cancel" => {
                    let return_to = state
                        .schedule_input_snapshot(chat_id)
                        .await
                        .map(|s| s.return_to.clone())
                        .unwrap_or_else(|| "s_add_menu".to_string());
                    state.remove_schedule_input(chat_id).await;
                    let new_q = q.clone();
                    q = CallbackQuery {
                        data: Some(return_to),
                        ..new_q
                    };
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 已取消自定义定时任务")
                        .await?;
                    continue;
                }
                d if d.starts_with("s_add:") => {
                    let template = d.strip_prefix("s_add:").unwrap_or(d);
                    let (task_type, cron) = match template {
                        "maint_sun_4" => (
                            logic::scheduler::task_types::TaskType::SystemMaintenance,
                            "0 4 * * Sun",
                        ),
                        "reboot_daily_3" => {
                            (logic::scheduler::task_types::TaskType::Reboot, "0 3 * * *")
                        }
                        "reload_daily_4" => (
                            logic::scheduler::task_types::TaskType::ReloadCore,
                            "0 4 * * *",
                        ),
                        _ => (
                            logic::scheduler::task_types::TaskType::SystemMaintenance,
                            "0 4 * * Sun",
                        ),
                    };

                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let task = logic::scheduler::ScheduledTask::new(task_type, cron);
                        let _ = manager
                            .add_new_task(bot.clone(), state.admin_id(), task)
                            .await;
                        bot.answer_callback_query(q.id.clone())
                            .text("✅ 任务添加成功")
                            .await?;

                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some("m_sched".to_string()),
                            ..new_q
                        };
                        continue;
                    }
                }
                "s_del_menu" => {
                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let state = manager.state.lock().await;
                        let mut buttons = Vec::new();
                        for (i, task) in state.tasks.iter().enumerate() {
                            buttons.push(vec![InlineKeyboardButton::callback(
                                format!("{}. {}", i + 1, task.task_type.get_display_name()),
                                format!("s_del:{}", i),
                            )]);
                        }
                        buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")]);
                        bot.edit_message_text(chat_id, msg_id, "➖ <b>删除任务</b>\n点击移除:")
                            .parse_mode(ParseMode::Html)
                            .reply_markup(InlineKeyboardMarkup::new(buttons))
                            .await?;
                    }
                }
                d if d.starts_with("s_del:") => {
                    let idx: usize = d.strip_prefix("s_del:").unwrap_or("0").parse().unwrap_or(0);

                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let state = manager.state.lock().await;
                        if let Err(e) = validate_idx(idx, state.tasks.len(), "任务") {
                            drop(state);
                            bot.answer_callback_query(q.id.clone())
                                .text(&format!("❌ {}", e))
                                .await?;
                            continue;
                        }
                        if let Some(task) = state.tasks.get(idx) {
                            let task_name = task.task_type.get_display_name().to_string();
                            drop(state); // Release lock before await

                            let keyboard = InlineKeyboardMarkup::new(vec![
                                vec![InlineKeyboardButton::callback(
                                    "⚠️ 确认删除",
                                    format!("s_del_confirm:{}", idx),
                                )],
                                vec![InlineKeyboardButton::callback("🔙 取消", "s_del_menu")],
                            ]);

                            bot.edit_message_text(
                            chat_id,
                            msg_id,
                            format!(
                                "⚠️ <b>删除确认</b>\n\n您确定要删除定时任务 <code>{}</code> 吗？",
                                escape_html(&task_name)
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                        } else {
                            drop(state);
                            bot.answer_callback_query(q.id)
                                .text("❌ 任务不存在")
                                .await?;
                        }
                    }
                }
                d if d.starts_with("s_del_confirm:") => {
                    let idx: usize = d
                        .strip_prefix("s_del_confirm:")
                        .unwrap()
                        .parse()
                        .unwrap_or(0);
                    if let Some(manager) = logic::scheduler::get_manager().await {
                        let _ = manager
                            .remove_task_at(bot.clone(), state.admin_id(), idx)
                            .await;
                        bot.answer_callback_query(q.id.clone())
                            .text("✅ 任务删除成功")
                            .show_alert(true)
                            .await?;

                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some("m_sched".to_string()),
                            ..new_q
                        };
                        continue;
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
        .context("token 包含无效的 UTF-8 字符")?
        .into();
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
