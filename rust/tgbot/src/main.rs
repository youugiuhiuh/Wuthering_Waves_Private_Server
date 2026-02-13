// mod logic; // Moved to lib.rs
use tgbot::logic;

use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use teloxide::utils::command::BotCommands;
use tgbot::logic::config::{ConfigManager, RealityProto, WarpMode};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::Operations;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::system::SystemMonitor;
use tgbot::logic::totp::TotpManager;
use tgbot::logic::upgrade::{
    UPGRADE_FLAG_FILE, UpgradeManager,
    wwps_core::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager},
};
use tokio::sync::Mutex;

const CONFIG_DIR: &str = "/etc/wwps/tgbot";
const KEY_FILE: &str = ".key";
const CONFIG_FILE: &str = "config.enc";
const BOT_VERSION: &str = env!("CARGO_PKG_VERSION");

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

#[derive(serde::Serialize, serde::Deserialize)]
struct EncryptedConfig {
    token: Vec<u8>,
    admin_id: Vec<u8>,
    totp_secret: Vec<u8>,
    #[serde(default)]
    self_destruct_key_hash: Option<String>,
}

async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context("无法向 Telegram 注册主命令")?;
    println!("✅ 已向 Telegram 注册主命令");
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
            buttons.push(vec![InlineKeyboardButton::callback(
                "🚀 双栈分离 (v6上v4下)",
                format!("{}s", ip_prefix),
            )]);
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
    ip_version: tgbot::logic::config::IpVersion,
    proto: RealityProto,
) -> ResponseResult<()> {
    let ip_ver_code = match ip_version {
        tgbot::logic::config::IpVersion::IPv4 => "4",
        tgbot::logic::config::IpVersion::IPv6 => "6",
        tgbot::logic::config::IpVersion::SplitStack => "s",
    };
    let ip_display = match ip_version {
        tgbot::logic::config::IpVersion::IPv4 => "IPv4",
        tgbot::logic::config::IpVersion::IPv6 => "IPv6",
        tgbot::logic::config::IpVersion::SplitStack => "双栈分离 (v6上v4下)",
    };

    let (exec_prefix, title) = match proto {
        RealityProto::Vision => ("u_batch_exec:", "Reality"),
        RealityProto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
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

struct AppState {
    admin_id: i64,
    totp_manager: TotpManager,
    sessions: Mutex<HashMap<i64, Instant>>,
    failed_attempts: Mutex<HashMap<i64, FailedRecord>>,
    pending_destructs: Mutex<HashMap<ChatId, DestructState>>,
    self_destruct_key_hash: Mutex<Option<String>>,
    pending_warp_inputs: Mutex<HashMap<ChatId, Instant>>,
}

struct DestructState {
    step: usize, // 1: Wait TOTP A, 2: Wait Confirm, 3: Wait TOTP B, 4: Wait File
    first_totp: String,
    second_totp: String,
    last_action_time: Instant,
}

struct FailedRecord {
    count: u32,
    first_fail: Instant,
    cooldown_until: Option<Instant>,
    lock_level: usize, // 0..3 对应 LOCKOUT_DURATIONS 索引
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

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "👋 欢迎使用 wwps 管理机器人！

请使用 /auth <验证码> 解锁 24 小时管理权限。",
            )
            .await?;
        }
        Command::Auth(code) => {
            let now = Instant::now();

            // 先检查是否处于冷却期
            {
                let mut fails = state.failed_attempts.lock().await;
                if let Some(rec) = fails.get_mut(&user_id)
                    && let Some(until) = rec.cooldown_until
                {
                    if until > now {
                        let remaining = until - now;
                        bot.send_message(
                            msg.chat.id,
                            format!(
                                "⚠️ 尝试过于频繁，请稍后再试。冷却剩余约 {} 分 {} 秒。",
                                remaining.as_secs() / 60,
                                remaining.as_secs() % 60
                            ),
                        )
                        .await?;
                        return Ok(());
                    } else {
                        rec.cooldown_until = None; // 冷却结束，允许继续
                    }
                }
            }

            if state.totp_manager.verify(&code) {
                state.sessions.lock().await.insert(user_id, now);
                state.failed_attempts.lock().await.remove(&user_id);
                bot.send_message(
                    msg.chat.id,
                    "✅ 认证成功！会话有效期 24 小时。请使用 /menu 开始管理。",
                )
                .await?;
            } else {
                let mut fails = state.failed_attempts.lock().await;
                let rec = fails.entry(user_id).or_insert(FailedRecord {
                    count: 0,
                    first_fail: now,
                    cooldown_until: None,
                    lock_level: 0,
                });

                // 如果超过窗口，重置计数窗口
                if now.duration_since(rec.first_fail) > TOTP_FAIL_WINDOW {
                    rec.count = 0;
                    rec.first_fail = now;
                    rec.cooldown_until = None;
                }

                rec.count += 1;
                rec.first_fail = rec.first_fail.min(now);

                if rec.count >= TOTP_FAIL_MAX {
                    // 获取当前锁定级别对应的时长
                    let duration = LOCKOUT_DURATIONS
                        .get(rec.lock_level)
                        .copied()
                        .unwrap_or(*LOCKOUT_DURATIONS.last().unwrap());

                    rec.cooldown_until = Some(now + duration);
                    rec.count = 0;
                    rec.first_fail = now;

                    // 升级锁定级别 (最高到 2，即 24h)
                    if rec.lock_level < LOCKOUT_DURATIONS.len() - 1 {
                        rec.lock_level += 1;
                    }

                    // 格式化时长显示
                    let duration_str = if duration.as_secs() >= 3600 {
                        format!("{} 小时", duration.as_secs() / 3600)
                    } else {
                        format!("{} 分钟", duration.as_secs() / 60)
                    };

                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ 验证失败次数过多，已进入冷却。\n⏱️ 锁定时间: {}\n⚠️ 请稍后再试。",
                            duration_str
                        ),
                    )
                    .await?;
                } else {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ TOTP 验证码无效，请检查后重试。（已失败 {} 次 / {} 次）",
                            rec.count, TOTP_FAIL_MAX
                        ),
                    )
                    .await?;
                }
            }
        }
        Command::SetSecurityFile => {
            if !is_authorized(&state, user_id).await {
                bot.send_message(msg.chat.id, "❌ 无权操作").await?;
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
                *state.self_destruct_key_hash.lock().await = Some(hash_hex.clone());

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
            if !is_authorized(&state, user_id).await {
                bot.send_message(msg.chat.id, "🔐 请先使用 /auth <验证码> 进行认证。")
                    .await?;
                return Ok(());
            }
            send_main_menu(bot, msg.chat.id).await?;
        }
    }

    Ok(())
}

async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    // Check if user is in self-destruct flow
    let timeout_check = {
        let mut destructs = state.pending_destructs.lock().await;
        if let Some(destruct_state) = destructs.get_mut(&chat_id) {
            // Check for timeout (60s)
            if destruct_state.last_action_time.elapsed() > Duration::from_secs(60) {
                Some(true)
            } else {
                // Update last action time for valid interactions
                destruct_state.last_action_time = Instant::now();
                Some(false)
            }
        } else {
            None
        }
    };

    // Check if user is in WARP input flow
    let warp_timeout_check = {
        let mut warp_inputs = state.pending_warp_inputs.lock().await;
        if let Some(start_time) = warp_inputs.get(&chat_id) {
            if start_time.elapsed() > Duration::from_secs(60) {
                Some(true)
            } else {
                Some(false)
            }
        } else {
            None
        }
    };

    if let Some(is_timeout) = warp_timeout_check {
        let mut warp_inputs = state.pending_warp_inputs.lock().await;
        warp_inputs.remove(&chat_id);

        if is_timeout {
            bot.send_message(chat_id, "⏳ 输入超时 (60s)，已自动取消。")
                .await?;
            return Ok(());
        } else {
            // Process input
            if let Some(text) = msg.text() {
                let rules: Vec<String> = text
                    .split(|c| c == ',' || c == '，' || c == '\n')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if rules.is_empty() {
                    bot.send_message(chat_id, "⚠️ 输入为空，请重新输入或使用 /menu 返回。")
                        .await?;
                    // Keep waiting (re-insert start time or just return to let them try again?
                    // Let's re-insert to reset timeout or just let them retry.
                    // Actually we removed it above. Let's re-add it if we want them to retry.
                    // Or just cancel. Let's cancel to avoid stuck loop.
                    return Ok(());
                }

                // Get current mode to preserve it
                let (_, current_mode) = ConfigManager::get_warp_routing_rules()
                    .await
                    .unwrap_or((Vec::new(), WarpMode::Default));

                match ConfigManager::update_warp_routing_rules(rules, current_mode).await {
                    Ok(_) => {
                        bot.send_message(chat_id, "✅ WARP 分流规则已更新并重载核心。")
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ 更新失败: {}", e))
                            .await?;
                    }
                }
            }
            return Ok(());
        }
    }

    if let Some(is_timeout) = timeout_check {
        if is_timeout {
            let mut destructs = state.pending_destructs.lock().await;
            destructs.remove(&chat_id);
            bot.send_message(chat_id, "⏳ 自毁流程超时 (60s)，已自动取消。")
                .await?;
            return Ok(());
        }

        // Use a new block to hold the lock for the rest of processing
        let mut destructs = state.pending_destructs.lock().await;
        if let Some(destruct_state) = destructs.get_mut(&chat_id) {
            // Enforce authorization for all steps
            if !is_authorized(&state, chat_id.0).await {
                bot.send_message(chat_id, "⚠️ 会话已过期，请重新认证")
                    .await?;
                return Ok(());
            }

            // Step 1: First TOTP verification
            if destruct_state.step == 1 {
                if let Some(text) = msg.text() {
                    let text = text.trim();
                    if state.totp_manager.verify(text) {
                        // Verify success, move to step 2 (Wait for confirmation button)
                        destruct_state.step = 2;
                        destruct_state.first_totp = text.to_string();

                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback(
                                "⚠️ 确认执行销毁",
                                "a_destroy_confirm",
                            )],
                            vec![InlineKeyboardButton::callback(
                                "🔙 取消",
                                "a_destroy_cancel",
                            )],
                        ]);

                        bot.send_message(
                        chat_id,
                        "⚠️ <b>危险操作确认 (2/4)</b>\n\n验证通过。\n请点击下方按钮确认执行销毁。\n此操作<b>不可逆</b>！",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                    } else {
                        bot.send_message(chat_id, "❌ 验证码错误，请重新输入。")
                            .await?;
                    }
                }
                return Ok(());
            }

            // Step 3: Second TOTP verification
            if destruct_state.step == 3 {
                if let Some(text) = msg.text() {
                    let text = text.trim();
                    if state.totp_manager.verify(text) {
                        // Replay protection A
                        if text == destruct_state.first_totp {
                            bot.send_message(chat_id, "❌ <b>安全警告 (防重放)</b>\n\n为了防止重放攻击，请等待下一个 TOTP 验证码（30秒刷新）。\n不能使用与第一次相同的验证码。").parse_mode(ParseMode::Html).await?;
                            return Ok(());
                        }

                        destruct_state.step = 4;
                        destruct_state.second_totp = text.to_string();

                        bot.send_message(
                         chat_id,
                         "🚨 <b>最终验证 (4/4)</b>\n\n请输入<b>安全验证文件</b> (图片或文档)。\n系统将比对文件指纹 (SHA-256) 以授权最终销毁。\n\n(如果没有设置安全文件，请使用 /set_security_file 先设置)",
                     ).parse_mode(ParseMode::Html).await?;
                    } else {
                        bot.send_message(chat_id, "❌ 验证码错误，请重新输入。")
                            .await?;
                    }
                }
                return Ok(());
            }

            // Step 4: Security File Verification
            if destruct_state.step == 4 {
                let (file_id, file_name) = if let Some(doc) = msg.document() {
                    (Some(doc.file.id.clone()), doc.file_name.clone())
                } else if let Some(photos) = msg.photo() {
                    (
                        photos.last().map(|p| p.file.id.clone()),
                        Some("图片".to_string()),
                    )
                } else {
                    (None, None)
                };

                if let Some(fid) = file_id {
                    let file = bot.get_file(fid.clone()).await?;
                    let mut content = Vec::new();
                    bot.download_file(&file.path, &mut content)
                        .await
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

                    // Calculate SHA-256
                    let mut hasher = Sha256::new();
                    hasher.update(&content);
                    let result = hasher.finalize();
                    let hash_hex = hex::encode(result);

                    let correct_hash = state.self_destruct_key_hash.lock().await.clone();
                    if let Some(correct) = correct_hash {
                        if hash_hex == correct {
                            let hash_short = if hash_hex.len() > 12 {
                                format!("{}...{}", &hash_hex[..8], &hash_hex[hash_hex.len() - 4..])
                            } else {
                                hash_hex.clone()
                            };
                            let file_display = file_name
                                .map(|n| format!("{} | {}", n, hash_short))
                                .unwrap_or_else(|| hash_short.clone());

                            // Verification Passed!
                            // Step 5: Final Button
                            destruct_state.step = 5;

                            let keyboard = InlineKeyboardMarkup::new(vec![
                                vec![InlineKeyboardButton::callback(
                                    "💀 最终确认销毁 (BOOM)",
                                    "a_destroy_final",
                                )],
                                vec![InlineKeyboardButton::callback(
                                    "🔙 取消",
                                    "a_destroy_cancel",
                                )],
                            ]);

                            bot.send_message(
                                chat_id,
                                format!(
                                    "☠️ <b>授权通过</b>\n\n指纹匹配成功 ({})。\n这是最后的确认，点击后服务器将<b>永久变砖</b>。",
                                    file_display
                                ),
                            )
                         .parse_mode(ParseMode::Html)
                         .reply_markup(keyboard)
                         .await?;
                        } else {
                            bot.send_message(chat_id, "❌ 文件验证失败。\nHash 不匹配。")
                                .await?;
                        }
                    } else {
                        bot.send_message(chat_id, "❌ 系统未设置安全验证文件，无法执行销毁。\n请先取消流程并使用 /set_security_file 设置文件。").await?;
                    }
                } else {
                    bot.send_message(chat_id, "⚠️ 请发送安全验证文件 (图片或文档)。")
                        .await?;
                }
                return Ok(());
            }
        }
    }

    Ok(())
}
async fn is_authorized(state: &Arc<AppState>, user_id: i64) -> bool {
    let sessions = state.sessions.lock().await;
    sessions
        .get(&user_id)
        .map(|t| t.elapsed() < Duration::from_secs(24 * 3600))
        .unwrap_or(false)
}

async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("📊 系统监控", "m_mon")],
        vec![InlineKeyboardButton::callback("👥 用户管理", "m_usr")],
        vec![InlineKeyboardButton::callback("📄 日志管理", "m_log")],
        vec![InlineKeyboardButton::callback("🛠 运维工具", "m_maint")],
        vec![InlineKeyboardButton::callback("⏰ 定时任务", "m_sched")],
    ]);
    bot.send_message(chat_id, "🏠 <b>主菜单</b>\n请选择操作类目:")
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        let user_id = q.from.id.0 as i64;
        if !is_authorized(&state, user_id).await {
            bot.answer_callback_query(q.id)
                .text("🚫 访问被拒绝，您没有权限执行此操作")
                .await?;
            return Ok(());
        }

        let data = match q.data.as_ref() {
            Some(d) => d.clone(),
            None => return Ok(()),
        };
        let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
        let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

        // Check for self-destruct timeout in callback
        let timeout_result = {
            let mut destructs = state.pending_destructs.lock().await;
            if let Some(state) = destructs.get_mut(&chat_id) {
                if state.last_action_time.elapsed() > Duration::from_secs(60) {
                    Some(true)
                } else {
                    state.last_action_time = Instant::now();
                    Some(false)
                }
            } else {
                None
            }
        };

        if let Some(is_timeout) = timeout_result {
            if is_timeout {
                let mut destructs = state.pending_destructs.lock().await;
                destructs.remove(&chat_id);
                bot.answer_callback_query(q.id)
                    .text("⏳ 流程已超时 (60s)")
                    .await?;
                bot.edit_message_text(chat_id, msg_id, "⏳ 自毁流程超时 (60s)，已自动取消。")
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(());
            }
        }

        match data.as_str() {
            "m_main" => {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback("📊 系统监控", "m_mon")],
                    vec![InlineKeyboardButton::callback("👥 用户管理", "m_usr")],
                    vec![InlineKeyboardButton::callback("📄 日志管理", "m_log")],
                    vec![InlineKeyboardButton::callback("🛠 运维工具", "m_maint")],
                    vec![InlineKeyboardButton::callback("🌩 WARP 分流", "m_warp")],
                ]);
                bot.edit_message_text(chat_id, msg_id, "🏠 <b>主菜单</b>\n选择操作:")
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
                    "{}\n\n🤖 <b>Bot 版本</b>: v{}\n\n⚙️ <b>核心进程</b>:\n- wwps-core: {}\n- wwps-box: {}",
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
                let inbounds = ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default();
                let mut buttons = Vec::new();

                if inbounds.is_empty() {
                    // 检查wwps配置目录是否存在
                    let wwps_core_config_exists = Path::new("/etc/wwps/wwps-core/conf/").exists();
                    let singbox_config_exists =
                        Path::new("/etc/wwps/wwps-box/conf/config/").exists();

                    if !wwps_core_config_exists && !singbox_config_exists {
                        // 完全没有安装wwps
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
                        // 已安装但没有找到inbounds配置文件
                        buttons.push(vec![
                            InlineKeyboardButton::callback("🚀 Reality 批量备份", "u_batch_init"),
                            InlineKeyboardButton::callback(
                                "🚀 Xhttp 批量备份",
                                "u_xhttp_batch_init",
                            ),
                        ]);
                        bot.edit_message_text(chat_id, msg_id,
                        "👥 <b>用户管理</b>\n\n⚠️ <b>未找到用户配置文件</b>\n\n检测到 wwps 已安装，但没有找到用户配置文件(*_inbounds.json)。\n\n您可以：\n• 创建 Reality 批量备份\n• 创建 Xhttp 批量备份\n• 检查配置文件是否正确放置")
                        .parse_mode(ParseMode::Html)
                        .reply_markup(InlineKeyboardMarkup::new(buttons))
                        .await?;
                    }
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
                        InlineKeyboardButton::callback("🚀 Xhttp 批量备份", "u_xhttp_batch_init"),
                    ]);
                    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "👥 <b>用户管理</b>\n选择配置文件 (支持批量删除):",
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
            }
            "m_log" => {
                let has_access = Path::new("/etc/wwps/wwps-core/access.log").exists();
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
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")],
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
            "m_maint" => {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![
                        InlineKeyboardButton::callback("🧹 系统维护", "a_sys_maint"),
                        InlineKeyboardButton::callback("🔄 重启系统", "a_sys_reboot"),
                    ],
                    vec![InlineKeyboardButton::callback("♻️ 重启核心", "a_reload")],
                    vec![
                        InlineKeyboardButton::callback("🚀 BBR+FQ", "a_bbr_fq"),
                        InlineKeyboardButton::callback("⚡ 1C1G 优化", "a_tune"),
                    ],
                    vec![
                        InlineKeyboardButton::callback("⚙️ 更新 Bot", "a_upgrade"),
                        InlineKeyboardButton::callback("🛰️ wwps-core 管理", "a_wwps_core_menu"),
                    ],
                    vec![
                        InlineKeyboardButton::callback("🌍 更新 GeoData", "a_geo"),
                        InlineKeyboardButton::callback("⏰ Geo 自动更新", "a_geo_sched_menu"),
                        InlineKeyboardButton::callback("🛡️ 安全加固", "a_fw"),
                    ],
                    vec![InlineKeyboardButton::callback("⚠️ 危险区域", "m_danger")],
                    vec![InlineKeyboardButton::callback("🌩 WARP 分流", "m_warp")],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")],
                ]);
                bot.edit_message_text(chat_id, msg_id, "🛠 <b>运维工具</b>\n高级系统维护指令:")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
            "m_danger" => {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "💥 立即自毁 (VPS过期一键删)",
                        "a_destroy_ask",
                    )],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
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
            "m_warp" => {
                let is_installed = WarpInstaller::is_installed().await;

                if !is_installed {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "🚀 安装 Cloudflare WARP",
                            "a_inst_warp",
                        )],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
                    ]);
                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        "⚠️ <b>未检测到 Cloudflare WARP</b>\n\n系统未安装 WARP 服务，无法配置分流规则。\n是否立即安装？"
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                } else {
                    let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                        .await
                        .unwrap_or((Vec::new(), WarpMode::Default));
                    let rule_display = if current_rules.is_empty() {
                        "<i>(无规则)</i>".to_string()
                    } else {
                        current_rules.join(", ")
                    };

                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback("✏️ 设置规则", "a_set_warp")],
                        vec![InlineKeyboardButton::callback(
                            format!("⚙️ 模式: {}", current_mode.as_str()),
                            "a_warp_switch_mode",
                        )],
                        vec![InlineKeyboardButton::callback("🗑️ 清除规则", "a_del_warp")],
                        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
                    ]);

                    bot.edit_message_text(
                        chat_id,
                        msg_id,
                        format!("🌩 <b>WARP 分流设置</b>\n\n当前模式: <b>{}</b>\n当前规则: {}\n\n说明: \n• 默认: 由 WARP 自动选择 IP\n• IPv4/IPv6: 强制 WARP 出口 IP 版本\n• 仅规则匹配的域名走 WARP。", current_mode.as_str(), rule_display)
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                }
            }
            "a_warp_switch_mode" => {
                let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                    .await
                    .unwrap_or((Vec::new(), WarpMode::Default));
                let next_mode = current_mode.next();

                match ConfigManager::update_warp_routing_rules(current_rules.clone(), next_mode)
                    .await
                {
                    Ok(_) => {
                        // Refresh view
                        let rule_display = if current_rules.is_empty() {
                            "<i>(无规则)</i>".to_string()
                        } else {
                            current_rules.join(", ")
                        };

                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback("✏️ 设置规则", "a_set_warp")],
                            vec![InlineKeyboardButton::callback(
                                format!("⚙️ 模式: {}", next_mode.as_str()),
                                "a_warp_switch_mode",
                            )],
                            vec![InlineKeyboardButton::callback("🗑️ 清除规则", "a_del_warp")],
                            vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
                        ]);

                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            format!("🌩 <b>WARP 分流设置</b>\n\n当前模式: <b>{}</b>\n当前规则: {}\n\n说明: \n• 默认: 由 WARP 自动选择 IP\n• IPv4/IPv6: 强制 WARP 出口 IP 版本\n• 仅规则匹配的域名走 WARP。", next_mode.as_str(), rule_display)
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                    }
                    Err(e) => {
                        bot.answer_callback_query(q.id)
                            .text(format!("❌ 切换失败: {}", e))
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

                        // Re-trigger m_warp to show config menu
                        let mut new_q = q.clone();
                        new_q.data = Some("m_warp".to_string());
                        // We need to pass the state recursively, or just let the user navigate back.
                        // But recursive call is cleaner for UX.
                        // However, handle_callback is async recursive? No, it returns BoxFuture.
                        // It's safe to call it.
                        return handle_callback(bot, new_q, state).await;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ <b>安装失败</b>\n原因: {}", e))
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
            }
            "a_set_warp" => {
                state
                    .pending_warp_inputs
                    .lock()
                    .await
                    .insert(chat_id, Instant::now());
                bot.send_message(
                    chat_id,
                    "✏️ <b>请输入分流规则</b>\n\n支持格式: `geosite:google, domain:reddit.com`\n多个规则请用逗号分隔。\n\n(请在 60 秒内输入)"
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
            "a_del_warp" => {
                match ConfigManager::update_warp_routing_rules(Vec::new(), WarpMode::Default).await
                {
                    Ok(_) => {
                        bot.answer_callback_query(q.id)
                            .text("✅ 规则已清除")
                            .await?;
                        // Refresh view
                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback("✏️ 设置规则", "a_set_warp")],
                            vec![InlineKeyboardButton::callback(
                                format!("⚙️ 模式: {}", WarpMode::Default.as_str()),
                                "a_warp_switch_mode",
                            )],
                            vec![InlineKeyboardButton::callback("🗑️ 清除规则", "a_del_warp")],
                            vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
                        ]);
                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            format!("🌩 <b>WARP 分流设置</b>\n\n当前模式: <b>{}</b>\n当前规则: <i>(无规则)</i>\n\n说明: \n• 默认: 由 WARP 自动选择 IP\n• IPv4/IPv6: 强制 WARP 出口 IP 版本\n• 仅规则匹配的域名走 WARP。", WarpMode::Default.as_str())
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                    }
                    Err(e) => {
                        bot.answer_callback_query(q.id)
                            .text(format!("❌ 清除失败: {}", e))
                            .await?;
                    }
                }
            }
            "a_destroy_ask" => {
                if !is_authorized(&state, chat_id.0).await {
                    bot.answer_callback_query(q.id)
                        .text("⚠️ 会话已过期，请重新认证")
                        .await?;
                    return Ok(());
                }
                // Step 1: Initialize destruction flow
                let mut destructs = state.pending_destructs.lock().await;
                destructs.insert(
                    chat_id,
                    DestructState {
                        step: 1,
                        first_totp: String::new(),
                        second_totp: String::new(),
                        last_action_time: Instant::now(),
                    },
                );

                // Clear TOTP failures to give user a clean slate for this critical op
                // (Optional, but user might be nervous and make mistakes)
                // state.failed_attempts.lock().await.remove(&user_id);

                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        "🔙 取消",
                        "a_destroy_cancel",
                    )]]);

                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "⚠️ <b>危险操作确认 (1/3)</b>\n\n您正在请求执行<b>自毁程序</b>。\n此操作将<b>彻底擦除</b>所有数据、配置及软件本身，且<b>不可恢复</b>。\n\n请输入 TOTP 验证码以继续:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            }
            "a_destroy_cancel" => {
                let mut destructs = state.pending_destructs.lock().await;
                if destructs.remove(&chat_id).is_some() {
                    bot.send_message(chat_id, "操作已取消。").await?;
                }
                // Return to danger menu
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "💥 立即自毁",
                        "a_destroy_ask",
                    )],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
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
            "a_destroy_confirm" => {
                if !is_authorized(&state, chat_id.0).await {
                    bot.answer_callback_query(q.id)
                        .text("⚠️ 会话已过期，请重新认证")
                        .await?;
                    return Ok(());
                }
                // Step 2 -> Step 3
                let mut destructs = state.pending_destructs.lock().await;
                if let Some(state) = destructs.get_mut(&chat_id) {
                    if state.step == 2 {
                        state.step = 3;

                        let keyboard =
                            InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                                "🔙 取消",
                                "a_destroy_cancel",
                            )]]);

                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "⚠️ <b>最终警告 (3/4)</b>\n\n请<b>再次输入新的 TOTP 验证码</b>以确认执行。\n(注意：必须与上一次验证码不同)",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                    } else {
                        // Invalid state transition
                        bot.answer_callback_query(q.id)
                            .text("状态无效，请重新开始")
                            .await?;
                    }
                } else {
                    bot.answer_callback_query(q.id)
                        .text("会话已过期，请重新开始")
                        .await?;
                }
            }
            "a_destroy_final" => {
                if !is_authorized(&state, chat_id.0).await {
                    bot.answer_callback_query(q.id)
                        .text("⚠️ 会话已过期，请重新认证")
                        .await?;
                    return Ok(());
                }
                let mut destructs = state.pending_destructs.lock().await;
                if let Some(state) = destructs.get_mut(&chat_id) {
                    if state.step == 5 {
                        bot.answer_callback_query(q.id.clone())
                            .text("正在执行销毁...")
                            .await?;

                        bot.edit_message_text(
                            chat_id,
                            msg_id,
                            "🚀 <b>最终验证通过。正在执行自毁程序...</b>\n\n所有数据将被擦除，Bot 将停止运行。\n再见。"
                         ).parse_mode(ParseMode::Html).await?;

                        tokio::task::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            if let Err(e) = MaintenanceManager::perform_self_destruct().await {
                                eprintln!("Self destruct failed: {}", e);
                            }
                        });
                        destructs.remove(&chat_id);
                    } else {
                        bot.answer_callback_query(q.id).text("状态无效").await?;
                    }
                }
            }
            "u_batch_init" => {
                if MaintenanceManager::is_reality_base_ready().await {
                    show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::Vision).await?;
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
                    show_reality_batch_prompt(&bot, chat_id, msg_id, RealityProto::XHTTP).await?;
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
                    ("u_batch_ip_init:", RealityProto::Vision)
                } else {
                    ("u_xhttp_batch_ip_init:", RealityProto::XHTTP)
                };
                let ip_ver_code = d.strip_prefix(prefix).unwrap();
                let ip_version = match ip_ver_code {
                    "6" => logic::config::IpVersion::IPv6,
                    "s" => logic::config::IpVersion::SplitStack,
                    _ => logic::config::IpVersion::IPv4,
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
                let parts: Vec<&str> = d.strip_prefix(prefix).unwrap().split(':').collect();
                if parts.len() != 2 {
                    return Ok(());
                }
                let ip_ver_code = parts[0]; // "4" or "6"
                let n: usize = parts[1].parse().unwrap_or(0);

                let ip_version = match ip_ver_code {
                    "6" => logic::config::IpVersion::IPv6,
                    "s" => logic::config::IpVersion::SplitStack,
                    _ => logic::config::IpVersion::IPv4,
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
                    logic::config::IpVersion::IPv4 => "IPv4",
                    logic::config::IpVersion::IPv6 => "IPv6",
                    logic::config::IpVersion::SplitStack => "双栈分离",
                };

                let proto_str = match proto {
                    RealityProto::Vision => "Reality",
                    RealityProto::XHTTP => "XHTTP",
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
                };

                match res {
                    Ok(result) => {
                        // 发送链接
                        let mut combined_links = String::new();
                        for (i, link) in result.links.iter().enumerate() {
                            combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                            if (i + 1) % 5 == 0 {
                                let _ = bot
                                    .send_message(chat_id, combined_links.clone())
                                    .parse_mode(ParseMode::Html)
                                    .await;
                                combined_links.clear();
                            }
                        }
                        if !combined_links.is_empty() {
                            bot.send_message(chat_id, combined_links)
                                .parse_mode(ParseMode::Html)
                                .await?;
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

                        bot.send_message(chat_id, result_msg).await?;
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
                let idx: usize = d.strip_prefix("u_l:").unwrap().parse().unwrap_or(0);
                let inbounds = ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default();
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
                            path.split('/').next_back().unwrap()
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
                }
            }
            // 删除特定用户逻辑
            d if d.starts_with("u_d:") => {
                let parts: Vec<&str> = d.strip_prefix("u_d:").unwrap().split(':').collect();
                if parts.len() == 2 {
                    let idx: usize = parts[0].parse().unwrap_or(0);
                    let email = parts[1];
                    let inbounds = ConfigManager::list_all_inbound_files()
                        .await
                        .unwrap_or_default();
                    if let Some(_path) = inbounds.get(idx) {
                        // TODO: Implement delete specific client in config if needed
                        // For now, let's just show a placeholder or handle file deletion if it's a standalone one
                        bot.answer_callback_query(q.id.clone())
                            .text(format!("🗑 暂不支持删除单个用户: {}", email))
                            .show_alert(true)
                            .await?;
                    }
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
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")],
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
                // 返回删除管理菜单
                let mut new_q = q.clone();
                new_q.data = Some("m_del_cfg".to_string());
                return handle_callback(bot, new_q, state).await;
            }
            // 按数量删除菜单
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
                    .unwrap()
                    .parse()
                    .unwrap_or(0);
                let deleted = ConfigManager::delete_configurations_by_count(n)
                    .await
                    .unwrap_or(0);
                bot.answer_callback_query(q.id.clone())
                    .text(format!("✅ 已成功清理 {} 个旧配置", deleted))
                    .show_alert(true)
                    .await?;
                // 返回删除管理菜单
                let mut new_q = q.clone();
                new_q.data = Some("m_del_cfg".to_string());
                return handle_callback(bot, new_q, state).await;
            }
            // 指定配置删除列表
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
            // 执行特定文件删除
            d if d.starts_with("cfg_del_file:") => {
                let idx: usize = d
                    .strip_prefix("cfg_del_file:")
                    .unwrap()
                    .parse()
                    .unwrap_or(0);
                let inbounds = ConfigManager::list_all_inbound_files()
                    .await
                    .unwrap_or_default();
                if let Some(path) = inbounds.get(idx) {
                    let _ = ConfigManager::delete_specific_configuration(path).await;
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 选择的文件已永久删除")
                        .show_alert(true)
                        .await?;
                }
                // 刷新选择菜单
                let mut new_q = q.clone();
                new_q.data = Some("cfg_del_select".to_string());
                return handle_callback(bot, new_q, state).await;
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
                            if let Err(err) = manager.run(bot_clone.clone(), chat_id_clone).await {
                                let _ = bot_clone
                                    .send_message(chat_id_clone, format!("❌ 自更新失败: {}", err))
                                    .await;
                            }
                        }
                        Err(err) => {
                            let _ = bot_clone
                                .send_message(chat_id_clone, format!("❌ 无法启动自更新: {}", err))
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
                                .send_message(chat_id_clone, format!("❌ GeoData 更新失败: {}", e))
                                .await;
                        }
                    }
                });

                bot.answer_callback_query(q.id)
                    .text("🌍 GeoData 已启动更新 (后台执行)")
                    .await?;
            }
            "a_geo_sched_menu" => {
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                let summary = if let Some(manager) = manager_guard.as_ref() {
                    manager.get_summary().await
                } else {
                    "❌ 调度器未初始化".to_string()
                };

                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "🟢 开启每日 01:35 更新",
                        "geo_sched_on",
                    )],
                    vec![InlineKeyboardButton::callback(
                        "⛔️ 停止 Geo 自动更新",
                        "geo_sched_off",
                    )],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
                ]);

                bot.edit_message_text(chat_id, msg_id, summary)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
            "geo_sched_on" => {
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                if let Some(manager) = manager_guard.as_ref() {
                    let task = logic::scheduler::ScheduledTask::new(
                        logic::scheduler::task_types::TaskType::GeoUpdate,
                        "35 1 * * *",
                    );
                    let _ = manager
                        .add_new_task(bot.clone(), state.admin_id, task)
                        .await;
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 已开启每日 01:35 GeoData 更新")
                        .await?;

                    let mut new_q = q.clone();
                    new_q.data = Some("m_sched".to_string());
                    return handle_callback(bot, new_q, state).await;
                } else {
                    bot.answer_callback_query(q.id)
                        .text("❌ 调度器未初始化")
                        .await?;
                }
            }
            "geo_sched_off" => {
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                if let Some(manager) = manager_guard.as_ref() {
                    let mut state_lock = manager.state.lock().await;
                    let mut removed = false;
                    for i in (0..state_lock.tasks.len()).rev() {
                        if state_lock.tasks[i].task_type
                            == logic::scheduler::task_types::TaskType::GeoUpdate
                        {
                            state_lock.tasks.remove(i);
                            removed = true;
                        }
                    }
                    let _ = state_lock.save_to_file(&manager.state_path);
                    drop(state_lock);
                    let _ = manager.start_all_tasks(bot.clone(), state.admin_id).await;

                    bot.answer_callback_query(q.id.clone())
                        .text(if removed {
                            "✅ 已停止 Geo 自动更新"
                        } else {
                            "ℹ️ 未找到 Geo 自动更新任务"
                        })
                        .await?;

                    let mut new_q = q.clone();
                    new_q.data = Some("m_sched".to_string());
                    return handle_callback(bot, new_q, state).await;
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
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_maint")],
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
                    if let Err(err) =
                        WwpsCoreUpgradeManager::run_upgrade(None, bot_clone.clone(), chat_id_clone)
                            .await
                    {
                        let _ = bot_clone
                            .send_message(chat_id_clone, format!("❌ wwps-core 升级失败: {}", err))
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
                            bot.edit_message_text(chat_id, msg_id, "未获取到可用版本，请稍后重试。")
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
                        .send_message(chat_id, "❌ 无法获取版本列表，请检查网络或 GitHub 访问。")
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
                            .send_message(chat_id_clone, format!("❌ wwps-core 升级失败: {}", err))
                            .await;
                    }
                });
            }
            "a_bbr_fq" => {
                let _ = MaintenanceManager::enable_bbr_fq().await;
                bot.answer_callback_query(q.id)
                    .text("✅ 已应用 BBR+FQ")
                    .await?;
            }
            "a_tune" => {
                let _ = MaintenanceManager::tune_vps_1c1g().await;
                bot.answer_callback_query(q.id)
                    .text("✅ 已应用 1C1G 优化")
                    .await?;
            }
            "a_sys_maint" => {
                bot.answer_callback_query(q.id.clone())
                    .text("🧹 正在执行系统维护...")
                    .await?;
                let bot_c = bot.clone();
                tokio::spawn(async move {
                    match Operations::perform_maintenance().await {
                        Ok(log) => {
                            // 防止日志过长，取最后4000字符
                            let log_tail = if log.len() > 4000 {
                                format!("... (Truncated)\n{}", &log[log.len() - 3000..])
                            } else {
                                log
                            };
                            let _ = bot_c
                                .send_message(
                                    chat_id,
                                    format!("✅ <b>系统维护完成</b>\n\n<pre>{}</pre>", log_tail),
                                )
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                        Err(e) => {
                            let _ = bot_c
                                .send_message(chat_id, format!("❌ <b>维护失败</b>\n\n原因: {}", e))
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                    }
                });
            }
            "a_sys_reboot" => {
                bot.answer_callback_query(q.id.clone())
                    .text("⚠️ 系统将于 3 秒后重启...")
                    .await?;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let _ = Operations::reboot_system().await;
                });
            }
            "m_sched" => {
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                let summary = if let Some(manager) = manager_guard.as_ref() {
                    manager.get_summary().await
                } else {
                    "❌ 调度器未初始化".to_string()
                };

                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![
                        InlineKeyboardButton::callback("➕ 添加任务", "s_add_menu"),
                        InlineKeyboardButton::callback("➖ 删除任务", "s_del_menu"),
                    ],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")],
                ]);

                bot.edit_message_text(chat_id, msg_id, summary)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
            "s_add_menu" => {
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        "每周日 4点 维护",
                        "s_add:maint_sun_4",
                    )],
                    vec![InlineKeyboardButton::callback(
                        "每天 3点 重启VPS",
                        "s_add:reboot_daily_3",
                    )],
                    vec![InlineKeyboardButton::callback(
                        "每天 4点 重启核心",
                        "s_add:reload_daily_4",
                    )],
                    vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")],
                ]);
                bot.edit_message_text(chat_id, msg_id, "➕ <b>添加快速任务</b>\n请选择预设模板:")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
            d if d.starts_with("s_add:") => {
                let template = d.strip_prefix("s_add:").unwrap();
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

                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                if let Some(manager) = manager_guard.as_ref() {
                    let task = logic::scheduler::ScheduledTask::new(task_type, cron);
                    let _ = manager
                        .add_new_task(bot.clone(), state.admin_id, task)
                        .await;
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 任务添加成功")
                        .await?;

                    // Refresh UI
                    let mut new_q = q.clone();
                    new_q.data = Some("m_sched".to_string());
                    return handle_callback(bot, new_q, state).await;
                }
            }
            "s_del_menu" => {
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                if let Some(manager) = manager_guard.as_ref() {
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
                let idx: usize = d.strip_prefix("s_del:").unwrap().parse().unwrap_or(0);
                let manager_guard = logic::scheduler::SCHEDULER.lock().await;
                if let Some(manager) = manager_guard.as_ref() {
                    let _ = manager
                        .remove_task_at(bot.clone(), state.admin_id, idx)
                        .await;
                    bot.answer_callback_query(q.id.clone())
                        .text("✅ 任务删除成功")
                        .await?;

                    // Refresh UI
                    let mut new_q = q.clone();
                    new_q.data = Some("m_sched".to_string());
                    return handle_callback(bot, new_q, state).await;
                }
            }
            _ => {
                bot.answer_callback_query(q.id).await?;
            }
        }
        Ok(())
    })
}

async fn save_config(state: &Arc<AppState>) -> Result<()> {
    let config_dir = Path::new(CONFIG_DIR);
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);

    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;

    let hash = state.self_destruct_key_hash.lock().await.clone();
    encrypted_config.self_destruct_key_hash = hash;

    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}

async fn run_setup(token: &str, admin_id: &str, totp_secret: &str) -> Result<()> {
    let config_dir = Path::new(CONFIG_DIR);
    fs::create_dir_all(config_dir)?;
    let security = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let encrypted_config = EncryptedConfig {
        token: security.encrypt(token)?,
        admin_id: security.encrypt(admin_id)?,
        totp_secret: security.encrypt(totp_secret)?,
        self_destruct_key_hash: None,
    };
    fs::write(
        config_dir.join(CONFIG_FILE),
        serde_json::to_vec(&encrypted_config)?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            config_dir.join(CONFIG_FILE),
            fs::Permissions::from_mode(0o600),
        )?;
    }
    println!("✅ Setup completed successfully.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "--setup" {
            if args.len() < 5 {
                println!("Usage: tgbot --setup <token> <admin_id> <totp_secret>");
                return Ok(());
            }
            return run_setup(&args[2], &args[3], &args[4]).await;
        } else if args[1] == "--generate-totp-secret" {
            println!("{}", TotpManager::generate_new_secret());
            return Ok(());
        } else if args[1] == "-v" || args[1] == "--version" {
            println!("tgbot {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    let config_dir = Path::new(CONFIG_DIR);
    let security =
        SecurityManager::new(&config_dir.join(KEY_FILE)).context("Security manager failed")?;
    let config_data = fs::read(config_dir.join(CONFIG_FILE)).context("Config file miss")?;
    let encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;

    let token = security.decrypt(&encrypted_config.token)?;
    let admin_id_secret = security.decrypt(&encrypted_config.admin_id)?;
    let totp_secret = security.decrypt(&encrypted_config.totp_secret)?;

    let admin_id: i64 = admin_id_secret
        .expose_secret()
        .trim()
        .parse()
        .context("Invalid admin_id format in config")?;

    let state = Arc::new(AppState {
        admin_id,
        totp_manager: TotpManager::new(&totp_secret)?,
        sessions: Mutex::new(HashMap::new()),
        failed_attempts: Mutex::new(HashMap::new()),
        pending_destructs: Mutex::new(HashMap::new()),
        self_destruct_key_hash: Mutex::new(encrypted_config.self_destruct_key_hash),
        pending_warp_inputs: Mutex::new(HashMap::new()),
    });

    let bot = Bot::new(token.expose_secret());
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

    let scheduler_state_path = config_dir
        .join("scheduler_state.json")
        .to_str()
        .unwrap()
        .to_string();
    logic::scheduler::init_scheduler(bot.clone(), admin_id, scheduler_state_path)
        .await
        .context("❌ 初始化调度器失败")?;

    let _ = notify_upgrade_success(&bot, admin_id).await;

    println!("🚀 Bot is starting...");
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
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
