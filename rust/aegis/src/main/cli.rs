use aegis::core::totp::TotpManager;
use anyhow::Result;

use crate::bootstrap::{run_setup, run_setup_from_stdin};

pub enum CliMode {
    Stdout(String),
    Setup {
        token: String,
        admin_id: String,
        totp_secret: String,
    },
    SetupStdin,
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
        "--setup" => {
            if args.len() < 5 {
                Some(CliMode::Stdout(
                    "Usage: aegis --setup <token> <admin_id> <totp_secret>".to_string(),
                ))
            } else {
                Some(CliMode::Setup {
                    token: args[2].clone(),
                    admin_id: args[3].clone(),
                    totp_secret: args[4].clone(),
                })
            }
        }
        "--setup-stdin" => Some(CliMode::SetupStdin),
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
        } => run_setup(&token, &admin_id, &totp_secret, None, None, None, None).await,
        CliMode::SetupStdin => run_setup_from_stdin().await,
    }
}
