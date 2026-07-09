// mod logic; // Moved to lib.rs
#![recursion_limit = "256"]
#![allow(clippy::vec_init_then_push)]
rust_i18n::i18n!("src/resources/i18n");

mod bootstrap;
#[path = "main/mod.rs"]
mod main;
mod utils;

use crate::bootstrap::{config_dir, harden_process, verify_integrity};
use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use aegis::app::state::AppState;
use aegis::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use aegis::core::security::self_destruct::production_executor;
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::system::upgrade::UPGRADE_FLAG_FILE;
use anyhow::Result;
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    harden_process();

    // 立即执行防调试检查
    aegis::core::security::anti_debug::check_debugger();

    // CLI 仅输出模式：先处理，避免 verify_integrity 向 stdout 打印导致安装器把整段输出当成 TOTP 密钥
    let args: Vec<String> = std::env::args().collect();
    if let Some(mode) = main::cli::try_cli_mode(&args) {
        return main::cli::execute_cli_mode(mode).await;
    }

    // 正常启动：校验完整性后再加载配置
    verify_integrity().await?;

    // CLI 模式检测（初始; auto-detect 补充在 encrypted_config 加载后）
    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_all = args.iter().any(|a| a == "--all");
    let mut enable_matrix = use_matrix || use_all;
    let enable_telegram = !use_matrix || use_all;

    let (app_config, security) = main::config::load_and_validate()?;

    // Auto-detect Matrix 配置
    let has_matrix = main::matrix::has_matrix_config(&app_config.decrypted.encrypted_config, &args);
    if !use_matrix && !use_all {
        enable_matrix = has_matrix && !args.iter().any(|a| a == "--tg-only");
    }

    let matrix_handle = if enable_matrix {
        Some(
            main::matrix::connect_matrix(
                &security,
                &app_config.decrypted.encrypted_config,
                &config_dir(),
            )
            .await?,
        )
    } else {
        None
    };

    let adapter = main::adapter::build_adapter(
        &app_config.decrypted.token,
        enable_telegram,
        enable_matrix,
        &matrix_handle,
    )
    .await?;

    let state = Arc::new(AppState::new(
        app_config.decrypted.admin_id,
        None,
        app_config.totp_manager,
        production_executor(),
        app_config
            .decrypted
            .encrypted_config
            .self_destruct_key_hash
            .clone(),
        app_config.bot_settings.session_timeout_secs,
        adapter,
    ));

    main::runtime::run(
        state,
        matrix_handle,
        enable_telegram,
        enable_matrix,
        app_config.decrypted.token,
        app_config.decrypted.admin_id,
    )
    .await
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
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

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
