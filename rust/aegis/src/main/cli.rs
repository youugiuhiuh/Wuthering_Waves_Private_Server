use aegis::core::totp::TotpManager;
use anyhow::{Context, Result};

use crate::bootstrap::{config_dir, run_setup, run_setup_from_stdin, set_simplex_admin_id};

pub enum CliMode {
    Stdout(String),
    Setup {
        token: Option<String>,
        admin_id: Option<String>,
        totp_secret: Option<String>,
    },
    SetupStdin,
    /// 补填 SimpleX 管理员 contactId。保留原始字符串，正整数校验在 bootstrap 层
    /// （那里才知道 id 的语义），与 `CliMode::Setup` 保留原始 token 的风格一致。
    SetSimplexAdmin(Option<String>),
    /// 本地审批旁路：列出在敲门的 SimpleX 联系人请求（不依赖 Telegram）。
    ListContactRequests,
    /// 批准请求。参数为 **contactRequestId**（不是 contactId）。
    ApproveContact(Option<String>),
    /// 拒绝请求（对请求方静默）。
    RejectContact(Option<String>),
    /// `args[1]` 是无法识别的 `-`/`--` 参数。
    ///
    /// 绝不放行：aegis 已作为 systemd 服务运行时，静默回落到正常启动会起第二个实例、
    /// 重复消费同一份事件流并重复执行命令。
    Unknown(String),
}

/// 解析 contactId 参数。仅做整数解析，非正数由 [`set_simplex_admin_id`] 拒绝。
pub fn parse_contact_id(raw: &str) -> Result<i64> {
    raw.trim()
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("contactId 必须是整数，收到 {raw:?}: {e}"))
}

pub fn try_cli_mode(args: &[String]) -> Option<CliMode> {
    if args.len() <= 1 {
        return None;
    }
    match args[1].as_str() {
        "--generate-totp-secret" => Some(CliMode::Stdout(TotpManager::generate_new_secret())),
        "-v" | "--version" => Some(CliMode::Stdout(format!(
            "aegis {}",
            env!("CARGO_PKG_VERSION")
        ))),
        "--setup" => Some(CliMode::Setup {
            token: args.get(2).cloned(),
            admin_id: args.get(3).cloned(),
            totp_secret: args.get(4).cloned(),
        }),
        "--setup-stdin" => Some(CliMode::SetupStdin),
        // 注意：缺参数时也必须返回 Some —— 返回 None 会让 main 继续正常启动 bot。
        "--set-simplex-admin" => Some(CliMode::SetSimplexAdmin(args.get(2).cloned())),
        "--list-contact-requests" => Some(CliMode::ListContactRequests),
        "--approve-contact" => Some(CliMode::ApproveContact(args.get(2).cloned())),
        "--reject-contact" => Some(CliMode::RejectContact(args.get(2).cloned())),
        // 合法但不走 CLI 模式的平台 flag：必须返回 None，让 main.rs 的
        // resolve_platform_selection 处理（其中 --discord 有专门的迁移指引报错）。
        "--simplex" | "--tg-simplex" | "--matrix" | "--all" | "--tg-only" | "--discord" => None,
        // 未知的 -/-- 参数一律报错。只校验 args[1]：--setup 的 args[2..] 是定位参数，
        // 可能是任意 token 字符串。
        other if other.starts_with('-') => Some(CliMode::Unknown(other.to_string())),
        _ => None,
    }
}

pub async fn execute_cli_mode(mode: CliMode) -> Result<()> {
    match mode {
        CliMode::Stdout(msg) => {
            println!("{msg}");
            Ok(())
        }
        CliMode::Setup {
            token,
            admin_id,
            totp_secret,
        } => {
            run_setup(
                token.as_deref(),
                admin_id.as_deref(),
                totp_secret.as_deref(),
                None,
                None,
                None,
                None,
            )
            .await
        }
        CliMode::SetupStdin => run_setup_from_stdin().await,
        CliMode::SetSimplexAdmin(raw) => {
            let raw = raw.context("用法: aegis --set-simplex-admin <contactId>")?;
            let admin_id = parse_contact_id(&raw)?;
            set_simplex_admin_id(&config_dir(), admin_id)
        }
        CliMode::ListContactRequests => {
            let pending = crate::main::simplex::list_pending_contact_requests().await?;
            if pending.is_empty() {
                println!("没有待批准的联系人请求");
            }
            for p in pending {
                println!(
                    "contactRequestId={} display_name={:?}",
                    p.contact_request_id, p.display_name
                );
            }
            Ok(())
        }
        CliMode::ApproveContact(raw) => {
            let raw = raw.context("用法: aegis --approve-contact <contactRequestId>")?;
            let id = parse_contact_id(&raw)?;
            let bound = crate::main::simplex::approve_contact_request(id).await?;
            println!("已批准联系人请求 contactRequestId={id}");
            if let Some(contact_id) = bound {
                println!(
                    "   contactId={contact_id}（可用 aegis --set-simplex-admin {contact_id} 绑定）"
                );
            }
            Ok(())
        }
        CliMode::RejectContact(raw) => {
            let raw = raw.context("用法: aegis --reject-contact <contactRequestId>")?;
            let id = parse_contact_id(&raw)?;
            crate::main::simplex::reject_contact_request(id).await?;
            println!("已拒绝联系人请求 contactRequestId={id}");
            Ok(())
        }
        CliMode::Unknown(flag) => Err(anyhow::anyhow!(
            "未知参数 {flag:?}。可用参数: --simplex | --tg-simplex | --matrix | --all | --tg-only | \
             --generate-totp-secret | --version | --setup <token> <admin_id> <totp_secret> | \
             --setup-stdin | --set-simplex-admin <contactId> | --list-contact-requests | \
             --approve-contact <contactRequestId> | --reject-contact <contactRequestId>"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_plain_contact_id() {
        assert_eq!(parse_contact_id("42").unwrap(), 42);
    }

    #[test]
    fn parses_contact_id_with_surrounding_whitespace() {
        assert_eq!(parse_contact_id("  42\t").unwrap(), 42);
    }

    #[test]
    fn rejects_non_numeric_contact_id() {
        assert!(parse_contact_id("abc").is_err());
        assert!(parse_contact_id("").is_err());
        assert!(parse_contact_id("42.0").is_err());
    }

    #[test]
    fn recognises_set_simplex_admin_with_value() {
        let mode = try_cli_mode(&args(&["aegis", "--set-simplex-admin", "42"]));
        assert!(matches!(mode, Some(CliMode::SetSimplexAdmin(Some(v))) if v == "42"));
    }

    /// 缺参数时仍必须返回 Some —— 返回 None 会让 main 继续正常启动 bot。
    #[test]
    fn recognises_set_simplex_admin_without_value() {
        let mode = try_cli_mode(&args(&["aegis", "--set-simplex-admin"]));
        assert!(matches!(mode, Some(CliMode::SetSimplexAdmin(None))));
    }

    #[test]
    fn recognises_contact_approval_flags() {
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--list-contact-requests"])),
            Some(CliMode::ListContactRequests)
        ));
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--approve-contact", "7"])),
            Some(CliMode::ApproveContact(Some(ref v))) if v == "7"
        ));
        // 缺参数仍须返回 Some（否则 main 会继续正常启动 bot）。
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--approve-contact"])),
            Some(CliMode::ApproveContact(None))
        ));
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--reject-contact", "7"])),
            Some(CliMode::RejectContact(Some(ref v))) if v == "7"
        ));
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--reject-contact"])),
            Some(CliMode::RejectContact(None))
        ));
    }

    /// aegis 已作为 systemd 服务运行时，静默回落到「正常启动」会起第二个实例、
    /// 消费同一份 WS 事件流并重复执行命令。拼写错误必须立刻可见。
    #[test]
    fn unknown_dash_argument_is_rejected_not_booted() {
        for arg in ["--set-simplex-admin=42", "--bogus", "-x", "--help"] {
            let mode = try_cli_mode(&args(&["aegis", arg]));
            assert!(
                matches!(mode, Some(CliMode::Unknown(ref f)) if f.as_str() == arg),
                "{arg} 必须被识别为未知参数，而不是回落到正常启动"
            );
        }
    }

    /// 平台 flag 必须穿透到 main.rs 的 resolve_platform_selection；
    /// `--discord` 保留穿透是为了让那条带迁移指引的专有报错继续生效。
    #[test]
    fn platform_flags_fall_through_to_normal_boot() {
        for arg in [
            "--simplex",
            "--tg-simplex",
            "--matrix",
            "--all",
            "--tg-only",
            "--discord",
        ] {
            assert!(
                try_cli_mode(&args(&["aegis", arg])).is_none(),
                "{arg} 必须返回 None"
            );
        }
    }

    /// `--setup` 的 args[2..] 是定位参数，可能是任意字符串（例如以 `--` 开头的 token）；
    /// 未知校验只针对 args[1]。
    #[test]
    fn setup_positional_args_are_not_treated_as_unknown_flags() {
        let mode = try_cli_mode(&args(&["aegis", "--setup", "--weird-token", "1", "SECRET"]));
        assert!(matches!(mode, Some(CliMode::Setup { .. })));
    }
}
