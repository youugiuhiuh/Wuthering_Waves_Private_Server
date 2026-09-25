// mod logic; // Moved to lib.rs
#![recursion_limit = "256"]
#![allow(clippy::vec_init_then_push)]
rust_i18n::i18n!("src/resources/i18n");

mod bootstrap;
#[path = "main/mod.rs"]
mod main;
mod utils;

use crate::bootstrap::{config_dir, harden_process, install_crypto_provider, verify_integrity};
use aegis::app::state::AppState;
use aegis::common::{BotAdapter, MessageContent, RoutingAdapter, TargetId};
use aegis::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use aegis::core::security::self_destruct::production_executor;
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::system::upgrade::UPGRADE_FLAG_FILE;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // 必须最先执行：无 logger 时 log::* 退化为 no-op，此处之前的任何日志都会被静默丢弃。
    main::logging::init_logger();

    harden_process();

    // 必须在任何 reqwest 0.13（matrix-sdk）Client 构建之前完成，否则 panic。
    install_crypto_provider();

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

    let adapter = if let Some(handle) = simplex_handle.as_ref().filter(|_| !selection.telegram) {
        // 纯 SimpleX：raw 适配器即主适配器（行为与 SimpleX standalone 接入时一致）。
        handle.adapter.clone()
    } else if let Some(handle) = simplex_handle.as_ref() {
        // Telegram + SimpleX：Telegram 为主，敏感内容改投 SimpleX 管理员联系人。
        // SimpleX 适配器用 parse_chat_id(target) 解读目标（Matrix 适配器忽略目标），
        // 因此必须把 secondary 目标改写为 simplex_admin_id，否则会发往 TG 的 chat id。
        let primary = main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?;
        // 独立 --simplex 形态允许无管理员启动（首次安装时 contactId 尚不存在）；
        // 但 TG+SimpleX 形态下 SimpleX 是敏感内容落点，必须已有投递目标，
        // 因此这里的 context 是真正的硬校验。
        let simplex_admin_id = app_config
            .decrypted
            .simplex_admin_id
            .context("启用 SimpleX 作为敏感内容落点时必须配置 simplex_admin_id")?;
        Arc::new(
            RoutingAdapter::new(primary, Some(handle.adapter.clone()))
                .with_secondary_target(TargetId(simplex_admin_id.to_string())),
        ) as Arc<dyn BotAdapter>
    } else {
        main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?
    };

    let state = AppState::new(
        app_config.decrypted.admin_id,
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
    );
    // 自愈开关：两种 SimpleX 形态都开启；平台串台由 `process_auth_code` 内的
    // `adapter.platform() == Platform::Simplex` 判据兜住（TG 的码不会重钉）。
    let state = if selection.simplex {
        state.with_simplex_repin()
    } else {
        state
    };
    let state =
        state.with_simplex_code_verified_for(app_config.bot_settings.simplex_code_verified_for);
    let state = Arc::new(state);

    // 同步 AppState 语言状态 + 一次性系统副作用（时区、apt-daily timer）。
    main::runtime::apply_configured_language(&state).await;

    main::runtime::run(
        state,
        matrix_handle,
        selection.telegram,
        selection.matrix,
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
/// | `--tg-simplex` | 任意 | 任意 | telegram + simplex（TG 主，敏感内容落 SimpleX） |
/// | `--all` | 任意 | 任意 | telegram + matrix（永不包含 simplex） |
/// | `--tg-only` | 任意 | 任意 | telegram（不做自动启用） |
///
/// 含 `--discord` 的参数组合直接报错（平台已移除）。
///
/// 自动探测（无 flag）最多只启用一个平台：`matrix_*` 与 `simplex_*` 同时齐备时
/// 直接报错，而不是悄悄二选一。
///
/// SimpleX 作为**唯一**平台时必须关闭 Telegram：SimpleX 适配器会成为主适配器，而
/// Telegram dispatcher 用同一个 `state.adapter` 派发，二者同开会把 Telegram 回复发到
/// SimpleX；无 token 时还会触发 `Bot::new` 的 panic。`--tg-simplex` 是唯一允许二者
/// 共存的形式：此时 Telegram 仍是主适配器，SimpleX 只作为敏感内容落点，且目标被
/// 改写为 `simplex_admin_id`（见 `main.rs` 中适配器装配处）。
fn resolve_platform_selection(
    args: &[String],
    has_matrix: bool,
    has_simplex: bool,
) -> Result<PlatformSelection, String> {
    // Discord 平台已移除：显式拒绝，而不是静默落到「无 flag → 自动探测」换平台启动。
    if args.iter().any(|a| a == "--discord") {
        return Err(
            "Discord 平台已移除，本版本不再支持 --discord。请改用 --matrix / --simplex / \
             --tg-only，或从 systemd 单元中移除 --discord。"
                .to_string(),
        );
    }

    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_simplex = args.iter().any(|a| a == "--simplex");
    let use_all = args.iter().any(|a| a == "--all");
    let tg_only = args.iter().any(|a| a == "--tg-only");
    let use_tg_simplex = args.iter().any(|a| a == "--tg-simplex");

    // `--tg-simplex` 必须先于 `--simplex` 判定：两者同时给出时取「TG 主 + SimpleX」，
    // 沿用既有「按判定顺序首个命中者胜」的先例（如 `--all --simplex` 取 simplex）。
    if use_tg_simplex {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: false,
            simplex: true,
        });
    }
    if use_simplex {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            simplex: true,
        });
    }
    if use_matrix {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: true,
            simplex: false,
        });
    }
    if use_all {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: true,
            simplex: false,
        });
    }
    if tg_only {
        return Ok(PlatformSelection {
            telegram: true,
            matrix: false,
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
            simplex: false,
        }),
        (false, true) => Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            simplex: true,
        }),
        (false, false) => Ok(PlatformSelection {
            telegram: true,
            matrix: false,
            simplex: false,
        }),
    }
}

/// 是否应当向 TG 推送 SimpleX 分享链接：仅 TG+SimpleX 且地址非空。
///
/// 抽成纯函数以便单测：调用点在 TG 分支，但判据本身与 IO 无关。
pub(crate) fn simplex_share_link_target(
    enable_telegram: bool,
    address: Option<&str>,
) -> Option<&str> {
    if !enable_telegram {
        return None;
    }
    address.map(str::trim).filter(|a| !a.is_empty())
}

/// 向 TG 管理员推送 SimpleX 分享链接（管理员点开即可连接本 bot）。
///
/// 强制走 `send_message_primary`：链接若走 `send_message`，会被敏感分流改投到
/// SimpleX，管理员反而在 TG 看不到。
async fn notify_simplex_share_link(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    address: &str,
) -> Result<()> {
    adapter
        .send_message_primary(
            target,
            MessageContent {
                text: rust_i18n::t!("simplex.share_link", "0" => address).to_string(),
                markup: None,
            },
        )
        .await?;
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
mod platform_selection_tests {
    use super::{PlatformSelection, resolve_platform_selection};

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn sel(telegram: bool, matrix: bool, simplex: bool) -> PlatformSelection {
        PlatformSelection {
            telegram,
            matrix,
            simplex,
        }
    }

    #[test]
    fn no_flags_no_config_is_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, false),
            Ok(sel(true, false, false))
        );
    }

    #[test]
    fn no_flags_matrix_config_is_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), true, false),
            Ok(sel(false, true, false))
        );
    }

    #[test]
    fn no_flags_simplex_config_is_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, true),
            Ok(sel(false, false, true))
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
            Ok(sel(false, true, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--matrix"]), true, true),
            Ok(sel(false, true, false))
        );
    }

    #[test]
    fn simplex_flag_suppresses_matrix_and_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, true),
            Ok(sel(false, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, false),
            Ok(sel(false, false, true))
        );
    }

    #[test]
    fn discord_flag_is_rejected() {
        let err = resolve_platform_selection(&v(&["--discord"]), false, false)
            .expect_err("--discord 必须被拒绝");
        assert!(
            err.contains("已移除"),
            "错误信息应说明平台已移除，实际: {err}"
        );

        let err = resolve_platform_selection(&v(&["--all", "--discord"]), true, true)
            .expect_err("--discord 与其它 flag 组合也必须被拒绝");
        assert!(
            err.contains("已移除"),
            "错误信息应说明平台已移除，实际: {err}"
        );
    }

    #[test]
    fn all_flag_is_telegram_plus_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), true, true),
            Ok(sel(true, true, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), false, false),
            Ok(sel(true, true, false))
        );
    }

    #[test]
    fn tg_only_flag_is_telegram_only() {
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-only"]), true, true),
            Ok(sel(true, false, false))
        );
    }

    #[test]
    fn all_with_simplex_prefers_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--simplex"]), false, false),
            Ok(sel(false, false, true))
        );
    }

    #[test]
    fn tg_simplex_flag_is_telegram_plus_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-simplex"]), false, false),
            Ok(sel(true, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-simplex"]), true, true),
            Ok(sel(true, false, true))
        );
    }

    #[test]
    fn tg_simplex_flag_wins_over_all_and_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--tg-simplex"]), true, true),
            Ok(sel(true, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex", "--tg-simplex"]), false, true),
            Ok(sel(true, false, true))
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

    /// 仅 TG+SimpleX 且真拿到地址时才向 TG 推送分享链接（纯 simplex 不发 TG）。
    #[test]
    fn share_link_only_when_tg_and_address_present() {
        let addr = "https://smp5.simplex.im/a#xyz";
        assert_eq!(simplex_share_link_target(true, Some(addr)), Some(addr));
        assert_eq!(simplex_share_link_target(true, Some("   ")), None);
        assert_eq!(simplex_share_link_target(true, None), None);
        assert_eq!(
            simplex_share_link_target(false, Some(addr)),
            None,
            "纯 simplex 不得往 TG 发"
        );
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
