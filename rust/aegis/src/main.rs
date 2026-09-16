// mod logic; // Moved to lib.rs
#![recursion_limit = "256"]
#![allow(clippy::vec_init_then_push)]
rust_i18n::i18n!("src/resources/i18n");

mod bootstrap;
#[path = "main/mod.rs"]
mod main;
mod utils;

use crate::bootstrap::{config_dir, harden_process, verify_integrity};
use aegis::app::state::AppState;
use aegis::common::{BotAdapter, MessageContent, TargetId};
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

    let (app_config, security) = main::config::load_and_validate()?;

    let has_matrix = main::matrix::has_matrix_config(&app_config.decrypted.encrypted_config, &args);
    let has_simplex =
        main::simplex::has_simplex_config(&app_config.decrypted.encrypted_config, &args);

    // 一次启动只选择一种平台部署形态（决策表见 resolve_platform_selection）。
    let selection = resolve_platform_selection(&args, has_matrix, has_simplex)
        .map_err(|e| anyhow::anyhow!("❌ {e}"))?;

    let matrix_handle = if selection.matrix {
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

    let discord_raw = if selection.discord {
        Some(
            main::discord::connect_discord(
                &security,
                &app_config.decrypted.encrypted_config,
                &config_dir(),
            )
            .await?,
        )
    } else {
        None
    };

    // 语言必须在连接 SimpleX 之前应用 —— connect_simplex 用 simplex.welcome 生成欢迎语，
    // 而该文案由进程全局 locale 决定。副作用（时区/apt timer）仍只在
    // runtime::apply_configured_language 中执行一次。
    if let Some(lang) = main::runtime::configured_lang() {
        aegis::core::i18n::set_lang(lang);
    }

    let simplex_handle = if selection.simplex {
        Some(
            main::simplex::connect_simplex(
                &security,
                &app_config.decrypted.encrypted_config,
                &config_dir(),
            )
            .await?,
        )
    } else {
        None
    };

    let adapter = if let Some(ref raw) = discord_raw {
        raw.adapter.clone()
    } else if let Some(ref handle) = simplex_handle {
        handle.adapter.clone()
    } else {
        main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?
    };

    let state = Arc::new(AppState::new(
        app_config.decrypted.admin_id,
        discord_raw.as_ref().map(|r| r.admin_id as i64),
        app_config.decrypted.simplex_admin_id,
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

    // 同步 AppState 语言状态 + 一次性系统副作用（时区、apt-daily timer）。
    main::runtime::apply_configured_language(&state).await;

    main::runtime::run(
        state,
        matrix_handle,
        selection.telegram,
        selection.matrix,
        discord_raw,
        simplex_handle,
        app_config.decrypted.token,
        app_config.decrypted.admin_id,
    )
    .await
}

/// 一次启动只选择一种平台部署形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlatformSelection {
    telegram: bool,
    matrix: bool,
    discord: bool,
    simplex: bool,
}

/// 依据显式 CLI 参数与配置存在性决定启用哪些平台。
///
/// `has_matrix` / `has_simplex` 表示 config.enc 中相应字段齐备（不含显式 flag）。
/// 决策表：
///
/// | 显式 flag | has_matrix | has_simplex | 结果 |
/// |---|---|---|---|
/// | 无 | 否 | 否 | telegram |
/// | 无 | 是 | 否 | matrix |
/// | 无 | 否 | 是 | simplex |
/// | 无 | 是 | 是 | 错误（配置歧义） |
/// | `--matrix` | 任意 | 任意 | matrix（压制 simplex 自动启用） |
/// | `--simplex` | 任意 | 任意 | simplex（压制 matrix 与 telegram） |
/// | `--discord` | 任意 | 任意 | discord（压制 matrix/simplex/telegram） |
/// | `--all` | 任意 | 任意 | telegram + matrix（永不包含 simplex/discord） |
/// | `--tg-only` | 任意 | 任意 | telegram（不做自动启用） |
/// | `--discord` + `--simplex` | 任意 | 任意 | 错误（两个独立平台） |
///
/// 自动探测（无 flag）最多只启用一个平台：`matrix_*` 与 `simplex_*` 同时齐备时
/// 直接报错，而不是悄悄二选一。SimpleX 成为主适配器时必须关闭 Telegram，否则
/// Telegram dispatcher 用同一个 `state.adapter` 派发，会把 Telegram 回复发到
/// SimpleX；无 token 时还会触发 `Bot::new` 的 panic。
fn resolve_platform_selection(
    args: &[String],
    has_matrix: bool,
    has_simplex: bool,
) -> Result<PlatformSelection, String> {
    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_discord = args.iter().any(|a| a == "--discord");
    let use_simplex = args.iter().any(|a| a == "--simplex");
    let use_all = args.iter().any(|a| a == "--all");
    let tg_only = args.iter().any(|a| a == "--tg-only");

    if use_discord && use_simplex {
        return Err(
            "--discord 与 --simplex 都是独立平台，不能同时启用。请只保留其一。".to_string(),
        );
    }

    if use_discord {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            discord: true,
            simplex: false,
        });
    }
    if use_simplex {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            discord: false,
            simplex: true,
        });
    }
    if use_matrix {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: true,
            discord: false,
            simplex: false,
        });
    }
    if use_all {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: true,
            discord: false,
            simplex: false,
        });
    }
    if tg_only {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: false,
            discord: false,
            simplex: false,
        });
    }

    match (has_matrix, has_simplex) {
        (true, true) => Err(
            "config.enc 同时存在 matrix_* 与 simplex_* 配置，无法自动选择平台。\
             请显式指定 --matrix 或 --simplex，或从 config.enc 中移除其中一份配置。"
                .to_string(),
        ),
        (true, false) => Ok(PlatformSelection {
            telegram: false,
            matrix: true,
            discord: false,
            simplex: false,
        }),
        (false, true) => Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            discord: false,
            simplex: true,
        }),
        (false, false) => Ok(PlatformSelection {
            telegram: true,
            matrix: false,
            discord: false,
            simplex: false,
        }),
    }
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
mod platform_selection_tests {
    use super::{PlatformSelection, resolve_platform_selection};

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn sel(telegram: bool, matrix: bool, discord: bool, simplex: bool) -> PlatformSelection {
        PlatformSelection {
            telegram,
            matrix,
            discord,
            simplex,
        }
    }

    #[test]
    fn no_flags_no_config_is_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, false),
            Ok(sel(true, false, false, false))
        );
    }

    #[test]
    fn no_flags_matrix_config_is_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), true, false),
            Ok(sel(false, true, false, false))
        );
    }

    #[test]
    fn no_flags_simplex_config_is_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, true),
            Ok(sel(false, false, false, true))
        );
    }

    #[test]
    fn no_flags_both_configs_is_error() {
        assert!(resolve_platform_selection(&v(&[]), true, true).is_err());
    }

    #[test]
    fn matrix_flag_wins_over_simplex_config() {
        assert_eq!(
            resolve_platform_selection(&v(&["--matrix"]), false, true),
            Ok(sel(false, true, false, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--matrix"]), true, true),
            Ok(sel(false, true, false, false))
        );
    }

    #[test]
    fn simplex_flag_suppresses_matrix_and_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, true),
            Ok(sel(false, false, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, false),
            Ok(sel(false, false, false, true))
        );
    }

    #[test]
    fn discord_flag_is_discord_only() {
        assert_eq!(
            resolve_platform_selection(&v(&["--discord"]), true, true),
            Ok(sel(false, false, true, false))
        );
    }

    #[test]
    fn all_flag_is_telegram_plus_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), true, true),
            Ok(sel(true, true, false, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), false, false),
            Ok(sel(true, true, false, false))
        );
    }

    #[test]
    fn tg_only_flag_is_telegram_only() {
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-only"]), true, true),
            Ok(sel(true, false, false, false))
        );
    }

    #[test]
    fn discord_and_simplex_flags_is_error() {
        assert!(resolve_platform_selection(&v(&["--discord", "--simplex"]), false, false).is_err());
    }

    #[test]
    fn all_with_simplex_prefers_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--simplex"]), false, false),
            Ok(sel(false, false, false, true))
        );
    }

    #[test]
    fn all_with_discord_prefers_discord() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--discord"]), false, false),
            Ok(sel(false, false, true, false))
        );
    }
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
