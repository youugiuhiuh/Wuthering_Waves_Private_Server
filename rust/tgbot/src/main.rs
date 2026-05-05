// mod logic; // Moved to lib.rs
mod app;
mod bootstrap;
mod handlers;

use crate::handlers::command::{Command, handle_command};
use crate::handlers::callback::handle_callback;
use crate::handlers::message::handle_message;

use obfstr::obfstr;

use tgbot::logic;

use crate::app::destruct_flow::{self, MessageFlowOutcome};
use crate::app::state::{AppState, TimeoutStatus};
use crate::bootstrap::{
    BotSettings, CONFIG_FILE, ConfigValidator,
    EncryptedConfig, KEY_FILE, config_dir, harden_process, run_setup, run_setup_from_stdin,
    verify_integrity,
};
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tgbot::logic::config::ConfigManager;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::self_destruct::production_executor;
use tgbot::logic::totp::TotpManager;





async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(Command::bot_commands())
        .await
        .context(obfstr!("无法向 Telegram 注册主命令").to_string())?;
    println!("{}", obfstr!("✅ 已向 Telegram 注册主命令"));
    Ok(())
}

const MAX_INPUT_LENGTH: usize = 4096;

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
    use tgbot::core::utils::format_duration_human;

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
}
