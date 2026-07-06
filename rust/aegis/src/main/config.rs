use std::fs;

use aegis::core::security::SecurityManager;
use aegis::core::totp::TotpManager;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;

use crate::bootstrap::{
    BotSettings, CONFIG_FILE, ConfigValidator, EncryptedConfig, KEY_FILE, config_dir,
};

pub struct DecryptedConfig {
    pub token: String,
    pub admin_id: i64,
    #[expect(dead_code)]
    pub totp_secret: String,
    pub encrypted_config: EncryptedConfig,
}

pub struct AppConfig {
    pub decrypted: DecryptedConfig,
    pub totp_manager: TotpManager,
    pub bot_settings: BotSettings,
}

pub fn load_and_validate() -> Result<(AppConfig, SecurityManager)> {
    let config_dir = config_dir();
    let key_path = config_dir.join(KEY_FILE);
    let config_path = config_dir.join(CONFIG_FILE);
    if config_path.exists() && !key_path.exists() {
        anyhow::bail!(
            "配置文件 {} 存在，但 {} 不存在。请将 setup 时生成的 .key 与 config.enc 一并部署到本机，或在本机重新执行 aegis --setup 完成初始化。",
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
        .context("token 包含无效的 UTF-8 字符")?;
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

    let totp_manager = TotpManager::new(&secrecy::SecretString::from(totp_secret.clone()))
        .map_err(|e| anyhow::anyhow!("初始化 TOTP 验证器失败: {}", e))?;

    let bot_settings = BotSettings::load();

    Ok((
        AppConfig {
            decrypted: DecryptedConfig {
                token,
                admin_id,
                totp_secret,
                encrypted_config,
            },
            totp_manager,
            bot_settings,
        },
        security,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn config_dir_uses_env_var_when_set() {
        let dir = TempDir::new().unwrap();
        // SAFETY: test-only env var manipulation, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
        }
        let result = crate::bootstrap::config_dir();
        assert_eq!(result, dir.path());
    }

    #[test]
    fn config_dir_defaults_when_env_not_set() {
        // SAFETY: test-only env var manipulation, single-threaded
        unsafe {
            std::env::remove_var("AEGIS_CONFIG_DIR");
        }
        let result = crate::bootstrap::config_dir();
        assert_eq!(result, std::path::PathBuf::from("/etc/wwps/aegis"));
    }

    #[test]
    fn load_and_validate_fails_when_config_exists_but_key_missing() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test-only env var manipulation, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains(".key") || err.contains("不存在"),
            "error should mention missing key: {err}"
        );
    }

    #[test]
    fn load_and_validate_fails_when_key_has_invalid_length() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test-only env var manipulation, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join(".key"), [0u8; 16]).unwrap();
        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
    }

    #[test]
    fn load_and_validate_fails_when_config_missing_and_key_exists() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test-only env var manipulation, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join(".key"), [0u8; 32]).unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
    }
}
