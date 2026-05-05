use crate::app::auth;
use crate::app::state::AppState;
use crate::bootstrap::{CONFIG_FILE, EncryptedConfig, KEY_FILE};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use teloxide::utils::command::BotCommands;
use tgbot::logic::security::SecurityManager;

const TOTP_FAIL_MAX: u32 = 5;
const TOTP_FAIL_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

const LOCKOUT_DURATIONS: [std::time::Duration; 4] = [
    std::time::Duration::from_secs(15 * 60),
    std::time::Duration::from_secs(60 * 60),
    std::time::Duration::from_secs(24 * 60 * 60),
    std::time::Duration::from_secs(48 * 60 * 60),
];

const MAX_FILE_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "支持以下命令:")]
pub enum Command {
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

pub fn looks_like_totp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

pub async fn process_auth_code(
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

pub async fn handle_command(
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

                let mut hasher = Sha256::new();
                hasher.update(&content);
                let result = hasher.finalize();
                let hash_hex = hex::encode(result);

                state
                    .set_self_destruct_key_hash(Some(hash_hex.clone()))
                    .await;

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

async fn save_config(state: &Arc<AppState>) -> Result<()> {
    let config_dir = crate::bootstrap::config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);

    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;

    let hash = state.self_destruct_key_hash().await;
    encrypted_config.self_destruct_key_hash = hash;

    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}
