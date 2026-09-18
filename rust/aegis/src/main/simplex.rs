use std::path::Path;
use std::sync::Arc;

use aegis::common::BotAdapter;
use aegis::core::security::SecurityManager;
use aegis::gateways::simplex::SimplexAdapter;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;

use crate::bootstrap::EncryptedConfig;

/// SimpleX runtime handle: WebSocket Bot, event stream, Adapter.
pub struct SimplexHandle {
    /// 与 `adapter` 内部持有的句柄指向同一连接；保留以便后续直接调用 Bot API。
    #[expect(dead_code)]
    pub bot: simploxide_client::ws::Bot,
    pub events: simploxide_client::ws::EventStream,
    pub adapter: Arc<dyn BotAdapter>,
}

/// 以 0600 原子写入 bot 地址文件：写 tmp → fsync → rename。
///
/// 权限显式设置两次 —— 建 tmp 时的 `.mode()` 对**已存在**的 tmp 不生效，而
/// `truncate(true)` 会复用它；上次崩溃残留的 tmp 若是 0644，rename 出去就是 0644。
///
/// 本任务的下一步（onboarding 批次 Task 2）会把它接线到 `connect_simplex`，
/// 届时移除该 `#[allow]`。注意不能用 `#[expect]`：测试 target 里该函数被测试使用，
/// `expect` 会变成 unfulfilled 而挂掉 `-D warnings`。
#[allow(dead_code)]
fn write_address_file(path: &Path, address: &str) -> std::io::Result<()> {
    use std::fs::Permissions;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        writeln!(file, "{address}")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}

pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--simplex");
    explicit
        || (encrypted_config.simplex_port.is_some() && encrypted_config.simplex_admin_id.is_some())
}

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

    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_address_file_writes_address_with_newline_and_0600() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd\n"
        );
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn write_address_file_truncates_previous_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, &"a".repeat(500)).unwrap();
        write_address_file(&path, "short").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short\n");
    }

    /// 只靠建文件时的 mode() 不够：它对已存在的 tmp 不生效，而 truncate(true) 会复用该文件。
    #[test]
    fn write_address_file_overrides_permissions_of_stale_tmp() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        let stale_tmp = dir.path().join("simplex_address.tmp");
        std::fs::write(&stale_tmp, b"leftover").unwrap();
        std::fs::set_permissions(&stale_tmp, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_address_file(&path, "simplex:/x").unwrap();

        assert_eq!(mode_of(&path), 0o600, "陈旧 tmp 的 0644 不得被 rename 出去");
        assert!(!stale_tmp.exists(), "tmp 应已被 rename 消费掉");
    }

    #[test]
    fn write_address_file_reports_error_when_parent_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("no-such-dir").join("simplex_address");
        assert!(write_address_file(&path, "simplex:/x").is_err());
    }
}
