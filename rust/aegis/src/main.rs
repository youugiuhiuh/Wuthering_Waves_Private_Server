// mod logic; // Moved to lib.rs
#![recursion_limit = "256"]
#![allow(clippy::vec_init_then_push)]
rust_i18n::i18n!("src/resources/i18n");

#[path = "adapters/telegram/handlers/mod.rs"]
mod handlers;

#[path = "adapters/matrix/handlers.rs"]
mod matrix_handlers;

mod app;
mod bootstrap;

mod utils;

use crate::handlers::menu;

use crate::app::auth;
use crate::app::state::AppState;
use crate::bootstrap::{
    BotSettings, CONFIG_FILE, ConfigValidator, EncryptedConfig, KEY_FILE, config_dir,
    harden_process, run_setup, run_setup_from_stdin, verify_integrity,
};
use aegis::adapters::common::{BotAdapter, MessageContent, RoutingAdapter, TargetId};
use aegis::adapters::matrix::MatrixAdapter;
use aegis::adapters::telegram::TelegramAdapter;
use aegis::core::i18n;
use aegis::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use aegis::core::security::SecurityManager;
use aegis::core::security::self_destruct::production_executor;
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::system::upgrade::UPGRADE_FLAG_FILE;
use aegis::core::totp::TotpManager;
use anyhow::{Context, Result};
use handlers::{callback, message};
use matrix_sdk::{
    Client as MatrixClient, Room, RoomState, config::SyncSettings,
    ruma::events::room::message::OriginalSyncRoomMessageEvent,
};
use obfstr::obfstr;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommands;

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

const MAX_FILE_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024;

async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context(obfstr!("无法向 Telegram 注册主命令").to_string())?;
    println!("{}", obfstr!("✅ 已向 Telegram 注册主命令"));
    Ok(())
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum Command {
    #[command(description = "Show help")]
    Help,
    #[command(description = "Start bot")]
    Start,
    #[command(description = "Show admin menu")]
    Menu,
    #[command(description = "Verify TOTP code")]
    Auth(String),
    #[command(description = "Set destruct verification file")]
    SetSecurityFile,
}

fn looks_like_totp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

async fn process_auth_code(
    state: &Arc<AppState>,
    target: &TargetId,
    user_id: i64,
    code: &str,
) -> anyhow::Result<bool> {
    auth::process_auth_code(
        &*state.adapter,
        target,
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
        bot.send_message(msg.chat.id, rust_i18n::t!("auth.invalid_user"))
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
                format!(
                    "{}\n\n{}",
                    rust_i18n::t!("welcome.title"),
                    rust_i18n::t!("welcome.prompt")
                ),
            )
            .await?;
        }
        Command::Auth(code) => {
            let target = TargetId(msg.chat.id.0.to_string());
            let _ = process_auth_code(&state, &target, user_id, &code).await;
        }
        Command::SetSecurityFile => {
            if !state.is_recently_authenticated(user_id).await {
                bot.send_message(msg.chat.id, rust_i18n::t!("auth.recent_auth_required"))
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
                        rust_i18n::t!("bot_commands.file_too_big", "0" => file.size, "1" => MAX_FILE_DOWNLOAD_SIZE),
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
                    rust_i18n::t!("bot_commands.security_file_set", "0" => hash_hex),
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            } else {
                bot.send_message(
                    msg.chat.id,
                    rust_i18n::t!("bot_commands.security_file_prompt"),
                )
                .await?;
            }
        }
        Command::Menu => {
            if !state.is_authorized(user_id).await {
                bot.send_message(msg.chat.id, rust_i18n::t!("auth.required"))
                    .await?;
                return Ok(());
            }
            menu::send_main_menu(bot, msg.chat.id).await?;
        }
    }

    Ok(())
}

const MAX_INPUT_LENGTH: usize = 4096;

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

pub(crate) async fn save_lang_to_config(_state: &Arc<AppState>, lang: i18n::Lang) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.lang = Some(lang.as_str().to_string());
    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    harden_process();

    // 立即执行防调试检查
    aegis::core::security::anti_debug::check_debugger();

    // CLI 仅输出模式：先处理，避免 verify_integrity 向 stdout 打印导致安装器把整段输出当成 TOTP 密钥
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "--generate-totp-secret" {
            println!("{}", TotpManager::generate_new_secret());
            return Ok(());
        }
        if args[1] == "-v" || args[1] == "--version" {
            println!("aegis {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        if args[1] == "--setup" {
            if args.len() < 5 {
                println!("Usage: aegis --setup <token> <admin_id> <totp_secret>");
                return Ok(());
            }
            return run_setup(&args[2], &args[3], &args[4], None).await;
        }
        if args[1] == "--setup-stdin" {
            return run_setup_from_stdin().await;
        }
    }

    // 正常启动：校验完整性后再加载配置
    verify_integrity().await?;

    // CLI 模式检测（初始; auto-detect 补充在 encrypted_config 加载后）
    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_all = args.iter().any(|a| a == "--all");
    let mut enable_matrix = use_matrix || use_all;
    let enable_telegram = !use_matrix || use_all;

    let config_dir = config_dir();
    let key_path = config_dir.join(KEY_FILE);
    let config_path = config_dir.join(CONFIG_FILE);
    if config_path.exists() && !key_path.exists() {
        anyhow::bail!(
            "配置文件 {} 存在，但 {} 不存在。请将 setup 时生成的 .key 与 config.enc 一并部署到本机，或在本机重新执行 aegis --setup 完成初始化。",
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

    // ── Auto-detect Matrix 配置 ──
    let has_matrix = encrypted_config.matrix_homeserver.is_some()
        && encrypted_config.matrix_username.is_some()
        && encrypted_config.matrix_password.is_some()
        && encrypted_config.matrix_room_id.is_some();
    if !use_matrix && !use_all {
        enable_matrix = has_matrix && !args.iter().any(|a| a == "--tg-only");
        // TG remains enabled by default (initialized as true above);
        // has_matrix only determines whether to add Matrix as secondary.
    }

    // ── Matrix 登录 ──
    let matrix_handle: Option<(MatrixClient, Room, Arc<dyn BotAdapter>)> = if enable_matrix {
        let decrypt_matrix = |field: &Option<Vec<u8>>| -> Result<String> {
            let vec = security.decrypt(field.as_ref().with_context(|| "缺少 Matrix 配置项")?)?;
            Ok(String::from_utf8(vec.expose_secret().to_vec())
                .map_err(|e| anyhow::anyhow!("Matrix 字段包含无效的 UTF-8: {}", e))?
                .trim()
                .to_string())
        };

        let matrix_homeserver = decrypt_matrix(&encrypted_config.matrix_homeserver)?;
        let matrix_username = decrypt_matrix(&encrypted_config.matrix_username)?;
        let matrix_pwd = decrypt_matrix(&encrypted_config.matrix_password)?;
        let matrix_room_id_str = decrypt_matrix(&encrypted_config.matrix_room_id)?;
        let matrix_store_passphrase = decrypt_matrix(&encrypted_config.matrix_store_passphrase)?;

        let store_path = config_dir.join("matrix_store");
        let client = MatrixClient::builder()
            .homeserver_url(&matrix_homeserver)
            .sqlite_store(&store_path, Some(&matrix_store_passphrase))
            .build()
            .await?;

        // ── Session restore (P0) ──
        let session_path = config_dir.join("matrix_session.json");
        if session_path.exists() {
            let session_json = fs::read_to_string(&session_path)?;
            let session: matrix_sdk::authentication::matrix::MatrixSession =
                serde_json::from_str(&session_json)?;
            client
                .matrix_auth()
                .restore_session(session, matrix_sdk::store::RoomLoadSettings::default())
                .await?;
            println!("✅ Matrix 会话恢复成功: {}", matrix_username);
        } else {
            client
                .matrix_auth()
                .login_username(&matrix_username, &matrix_pwd)
                .initial_device_display_name("Aegis Matrix Bot")
                .send()
                .await?;
            println!("✅ Matrix 登录成功: {}", matrix_username);

            // Save session for future restarts
            if let Some(session) = client.matrix_auth().session() {
                let session_json = serde_json::to_string(&session)?;
                fs::write(&session_path, session_json)?;
                println!("✅ Matrix 会话已保存");
            }
        }

        // P2: Bootstrap cross-signing (best-effort)
        use matrix_sdk::ruma::api::client::uiaa::{
            AuthData, MatrixUserIdentifier, Password, UserIdentifier,
        };
        if client
            .encryption()
            .bootstrap_cross_signing_if_needed(None)
            .await
            .is_err()
        {
            let _ = client
                .encryption()
                .bootstrap_cross_signing_if_needed(Some(AuthData::Password(Password::new(
                    UserIdentifier::Matrix(MatrixUserIdentifier::new(matrix_username.clone())),
                    matrix_pwd.clone(),
                ))))
                .await;
        }

        // P1: Wait for E2EE initialization tasks
        client
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;

        client.sync_once(SyncSettings::default()).await?;

        let room_id: matrix_sdk::ruma::OwnedRoomId = matrix_room_id_str.parse()?;

        let client_inv = client.clone();
        client.add_event_handler(
            move |_: matrix_sdk::ruma::events::room::member::OriginalSyncRoomMemberEvent,
                  room: Room| {
                let c = client_inv.clone();
                async move {
                    if room.state() == RoomState::Invited {
                        let _ = c.join_room_by_id(room.room_id()).await;
                    }
                }
            },
        );

        let room = client
            .get_room(&room_id)
            .context("未找到 Matrix 房间，请先邀请机器人到房间")?;

        let matrix_adapter: Arc<dyn BotAdapter> = Arc::new(MatrixAdapter::new(room.clone()));
        Some((client, room, matrix_adapter))
    } else {
        None
    };

    // ── 创建主适配器与 AppState ──
    let adapter: Arc<dyn BotAdapter> = if enable_telegram && enable_matrix {
        // 双通道：RoutingAdapter
        let tg_adapter = {
            let bot = Bot::new(&token);
            if let Err(err) = register_bot_commands(&bot).await {
                eprintln!("[WARN] 命令注册失败: {}", err);
            }
            Arc::new(TelegramAdapter::new(bot)) as Arc<dyn BotAdapter>
        };
        let secondary = matrix_handle.as_ref().map(|(_, _, a)| a.clone());
        Arc::new(RoutingAdapter::new(tg_adapter, secondary))
    } else if enable_telegram {
        let bot = Bot::new(&token);
        if let Err(err) = register_bot_commands(&bot).await {
            eprintln!("[WARN] 命令注册失败: {}", err);
        }
        Arc::new(TelegramAdapter::new(bot))
    } else if let Some((_, _, ref matrix_adapter)) = matrix_handle {
        matrix_adapter.clone()
    } else {
        anyhow::bail!("没有启用任何平台，请使用 --matrix 或 --all 或省略参数使用 Telegram");
    };

    let state = Arc::new(AppState::new(
        admin_id,
        totp_manager_instance,
        production_executor(),
        encrypted_config.self_destruct_key_hash.clone(),
        bot_settings.session_timeout_secs,
        adapter,
    ));

    // Initialize i18n language from config
    if let Some(ref lang_str) = encrypted_config.lang {
        let lang = lang_str.parse().unwrap_or(i18n::Lang::Zh);
        i18n::set_lang(lang);
        state.set_lang(lang).await;
        state.mark_lang_configured().await;
    }

    // ── Matrix 同步循环 ──
    if let Some((client, room, matrix_adapter)) = matrix_handle {
        let target = TargetId(room.room_id().to_string());

        fn parse_user_id(s: &str) -> i64 {
            s.trim_start_matches('@')
                .split(':')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        }

        let matrix_state = state.clone();
        let matrix_adapter_sync = matrix_adapter;
        let matrix_target = target.clone();

        client.add_event_handler(
            move |event: OriginalSyncRoomMessageEvent, room: Room, _client: MatrixClient| {
                let state = matrix_state.clone();
                let adapter = matrix_adapter_sync.clone();
                let target = matrix_target.clone();
                async move {
                    if room.room_id().as_str() != target.0.as_str() {
                        return;
                    }
                    let user_id = parse_user_id(event.sender.as_str());
                    if !state.is_admin_user(user_id) {
                        return;
                    }
                    let text = event.content.body().trim().to_string();

                    if crate::looks_like_totp_code(&text) && !state.is_authorized(user_id).await {
                        let _ = crate::process_auth_code(&state, &target, user_id, &text).await;
                        return;
                    }

                    let cmd = aegis::adapters::matrix::commands::parse(&text);
                    if !matches!(cmd, aegis::adapters::matrix::commands::Command::Auth { .. }) {
                        let _ = matrix_handlers::dispatch(&cmd, &*adapter, &target, &state).await;
                    }
                }
            },
        );

        tokio::spawn(async move {
            if let Err(e) = client.sync(SyncSettings::default()).await {
                log::error!("Matrix sync error: {}", e);
            }
        });
    }

    // ── Telegram Dispatcher ──
    if enable_telegram {
        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint(handle_command),
            )
            .branch(Update::filter_message().endpoint(message::handle_message))
            .branch(Update::filter_callback_query().endpoint(callback::handle_callback));

        let adapter_for_init = state.adapter.clone();
        let target_for_init = TargetId(admin_id.to_string());
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            let _ = notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
            let _ = notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init).await;
            let _ = notify_online(&*adapter_for_init, &target_for_init).await;
        });

        Dispatcher::builder(Bot::new(&token), handler)
            .dependencies(dptree::deps![state.clone()])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    }

    // ── Matrix-only: 后台初始化 + 保活 ──
    if enable_matrix && !enable_telegram {
        let adapter_for_init = state.adapter.clone();
        let target_for_init = TargetId(admin_id.to_string());
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            let _ = notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
            let _ = notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init).await;
            let _ = notify_online(&*adapter_for_init, &target_for_init).await;
        });

        // 保活 — matrix sync runs in background via spawn above
        let () = std::future::pending().await;
    }

    Ok(())
}

async fn notify_online(adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
    let ip = match SystemMonitor::get_public_ip().await {
        Ok(ip) => ip,
        Err(err) => {
            log::warn!("获取公网 IPv4 失败: {}", err);
            "Unavailable".to_string()
        }
    };

    let masked_ip = if ip.contains('.') {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 {
            format!("{}.{}.*.*", parts[0], parts[1])
        } else {
            ip.clone()
        }
    } else {
        ip.clone()
    };

    let sys_info = "Linux";

    let msg = rust_i18n::t!("system.online", "0" => masked_ip, "1" => sys_info);

    let _ = adapter
        .send_message(
            target,
            MessageContent {
                text: msg.into_owned(),
                markup: None,
            },
        )
        .await;
    Ok(())
}

async fn notify_upgrade_success(adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
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
        rust_i18n::t!("system.upgrade_done_no_version")
    } else {
        rust_i18n::t!("system.upgrade_success", "0" => version)
    };

    adapter
        .send_message(
            target,
            MessageContent {
                text: message.into_owned(),
                markup: None,
            },
        )
        .await?;
    Ok(())
}

async fn notify_bbr3_reboot_result(adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
    let flag_path = Path::new(BBR3_PENDING_FLAG_FILE);
    if !flag_path.exists() {
        return Ok(());
    }

    let info = MaintenanceManager::collect_bbr3_runtime_info().await;

    if let Err(e) = fs::remove_file(flag_path) {
        eprintln!("[WARN] 无法删除 BBR3 标记文件: {}", e);
    }

    let kernel_hint = if info.has_xanmod_kernel {
        rust_i18n::t!("system.yes")
    } else {
        rust_i18n::t!("system.no")
    };
    let proc_hint = if info.has_xanmod_proc_version {
        rust_i18n::t!("system.yes")
    } else {
        rust_i18n::t!("system.no")
    };

    let message = rust_i18n::t!(
        "system.bbr3_check_result",
        "0" => info.uname_r, "1" => info.tcp_congestion_control, "2" => info.proc_version,
        "3" => kernel_hint, "4" => proc_hint
    );

    adapter
        .send_message(
            target,
            MessageContent {
                text: message.into_owned(),
                markup: None,
            },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::format_duration_human;
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use anyhow::Result;
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

        aegis::core::security::self_destruct::trigger(executor);
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
