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
}
