// mod logic; // Moved to lib.rs
mod app;
mod bootstrap;
mod handlers;

use crate::handlers::command::{Command, handle_command, looks_like_totp_code, process_auth_code};
use crate::handlers::proxy::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};
use crate::handlers::system::{
    build_custom_schedule_text, build_custom_schedule_keyboard, build_custom_day_keyboard,
    build_custom_hour_keyboard, build_custom_minute_keyboard, build_custom_timezone_keyboard,
    build_cron_from_custom_state,
};
use crate::handlers::callback::handle_callback;

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
use tgbot::logic::config::{ConfigManager, KcpMask, Proto, WarpMode};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::task_types::TaskType;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::self_destruct::production_executor;
use tgbot::logic::system::SystemMonitor;
use tgbot::logic::totp::TotpManager;
use tgbot::logic::upgrade::{
    UpgradeManager,
    wwps_core::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager},
};
use tgbot::logic::log_audit::{LogAudit, SERVICE_WWPS_CORE, SERVICE_SING_BOX};





async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context(obfstr!("无法向 Telegram 注册主命令").to_string())?;
    println!("{}", obfstr!("✅ 已向 Telegram 注册主命令"));
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
        let _ = crate::app::lifecycle::notify_upgrade_success(&bot_for_init, admin_id).await;
        let _ = crate::app::lifecycle::notify_bbr3_reboot_result(&bot_for_init, admin_id).await;
        let _ = crate::app::lifecycle::notify_online(&bot_for_init, admin_id).await;
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
