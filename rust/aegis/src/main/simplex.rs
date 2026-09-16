use std::path::Path;
use std::sync::Arc;

use aegis::common::BotAdapter;
use aegis::core::security::SecurityManager;
use aegis::gateways::simplex::SimplexAdapter;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;

use crate::bootstrap::EncryptedConfig;

/// SimpleX runtime handle: WebSocket Bot, event stream, Adapter.
/// 接线在 Task 7（main.rs / runtime.rs）完成前，本模块暂未被引用。
#[allow(dead_code)]
pub struct SimplexHandle {
    pub bot: simploxide_client::ws::Bot,
    pub events: simploxide_client::ws::EventStream,
    pub adapter: Arc<dyn BotAdapter>,
}

#[allow(dead_code)]
pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--simplex");
    explicit
        || (encrypted_config.simplex_port.is_some() && encrypted_config.simplex_admin_id.is_some())
}

#[allow(dead_code)]
pub async fn connect_simplex(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
    _config_dir: &Path,
) -> Result<SimplexHandle> {
    let decrypt = |field: &Option<Vec<u8>>, what: &str| -> Result<String> {
        let vec = security.decrypt(field.as_ref().with_context(|| format!("缺少 {what}"))?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("{what} 包含无效 UTF-8: {e}"))?
            .trim()
            .to_string())
    };

    let port: u16 = decrypt(&encrypted_config.simplex_port, "simplex_port")?
        .parse()
        .context("simplex_port 应为 1-65535 的整数")?;
    let _admin_id: i64 = decrypt(&encrypted_config.simplex_admin_id, "simplex_admin_id")?
        .parse()
        .context("simplex_admin_id 应为整数 contactId")?;

    let (bot, events) = simploxide_client::ws::BotBuilder::new("Aegis", port)
        .auto_accept_with(rust_i18n::t!("simplex.welcome").to_string())
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("连接 SimpleX WebSocket 失败: {e}"))?;

    let adapter: Arc<dyn BotAdapter> = Arc::new(SimplexAdapter::new(bot.clone()));
    Ok(SimplexHandle {
        bot,
        events,
        adapter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> EncryptedConfig {
        EncryptedConfig {
            token: None,
            admin_id: None,
            totp_secret: None,
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
            simplex_port: None,
            simplex_admin_id: None,
        }
    }

    #[test]
    fn returns_false_when_no_simplex_config() {
        assert!(!has_simplex_config(&empty_config(), &[]));
    }

    #[test]
    fn returns_true_when_flag_present() {
        assert!(has_simplex_config(
            &empty_config(),
            &["--simplex".to_string()]
        ));
    }

    #[test]
    fn returns_true_when_both_fields_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        cfg.simplex_admin_id = Some(vec![1]);
        assert!(has_simplex_config(&cfg, &[]));
    }

    #[test]
    fn returns_false_when_only_port_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        assert!(!has_simplex_config(&cfg, &[]));
    }
}
