use std::fs;

use aegis::core::security::SecurityManager;
use aegis::core::totp::TotpManager;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
#[cfg(test)]
use serial_test::serial;

use crate::bootstrap::{
    BotSettings, CONFIG_FILE, ConfigValidator, EncryptedConfig, KEY_FILE, config_dir,
};

pub struct DecryptedConfig {
    pub token: Option<String>,
    pub admin_id: Option<i64>,
    #[expect(dead_code)]
    pub discord_token: Option<String>,
    #[expect(dead_code)]
    pub discord_admin_id: Option<i64>,
    pub encrypted_config: EncryptedConfig,
}

pub struct AppConfig {
    pub decrypted: DecryptedConfig,
    pub totp_manager: Option<TotpManager>,
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

    let token = match &encrypted_config.token {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 token 失败")?;
            Some(
                String::from_utf8(vec.expose_secret().to_vec())
                    .context("token 包含无效的 UTF-8 字符")?,
            )
        }
        None => None,
    };

    let admin_id = match &encrypted_config.admin_id {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 admin_id 失败")?;
            let s = String::from_utf8(vec.expose_secret().to_vec())
                .context("admin_id 包含无效的 UTF-8 字符")?;
            Some(
                s.trim()
                    .parse()
                    .context("无效的 admin_id 格式 (应为 i64)")?,
            )
        }
        None => None,
    };

    // totp_secret：用完即焚——仅用于构建 TotpManager，不存入任何驻留字段。
    // SecretString 局部变量出作用域即清零。
    let totp_secret: Option<secrecy::SecretString> = match &encrypted_config.totp_secret {
        Some(v) => Some(
            security
                .decrypt_secret(v)
                .context("解密 totp_secret 失败")?,
        ),
        None => None,
    };

    let discord_token = match &encrypted_config.discord_token {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 discord_token 失败")?;
            Some(
                String::from_utf8(vec.expose_secret().to_vec())
                    .map_err(|e| anyhow::anyhow!("discord_token 包含无效的 UTF-8: {}", e))?
                    .trim()
                    .to_string(),
            )
        }
        None => None,
    };
    let discord_admin_id = match &encrypted_config.discord_admin_id {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 discord_admin_id 失败")?;
            let s = String::from_utf8(vec.expose_secret().to_vec())
                .map_err(|e| anyhow::anyhow!("discord_admin_id 包含无效的 UTF-8: {}", e))?
                .trim()
                .to_string();
            Some(s.parse::<i64>().context("discord_admin_id 应为整数")?)
        }
        None => None,
    };

    let validator = ConfigValidator::new();
    if let Err(e) = validator.validate_decrypted_config(
        token.as_deref(),
        admin_id,
        totp_secret.as_ref().map(|s| s.expose_secret().as_str()),
        &encrypted_config.self_destruct_key_hash,
    ) {
        anyhow::bail!("❌ 配置校验失败: {}", e);
    }

    let totp_manager = match totp_secret.as_ref() {
        Some(secret) => Some(
            TotpManager::new(secret)
                .map_err(|e| anyhow::anyhow!("初始化 TOTP 验证器失败: {}", e))?,
        ),
        None => None,
    };
    // 用完即焚：TotpManager 已持有受保护副本，明文局部立即清零释放，不再驻留整个进程
    drop(totp_secret);

    let bot_settings = BotSettings::load();

    Ok((
        AppConfig {
            decrypted: DecryptedConfig {
                token,
                admin_id,
                discord_token,
                discord_admin_id,
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

    #[serial]
    #[test]
    fn config_dir_uses_env_var_when_set() {
        let dir = TempDir::new().unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
        }
        let result = crate::bootstrap::config_dir();
        assert_eq!(result, dir.path());
    }

    #[serial]
    #[test]
    fn config_dir_defaults_when_env_not_set() {
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::remove_var("AEGIS_CONFIG_DIR");
        }
        let result = crate::bootstrap::config_dir();
        assert_eq!(result, std::path::PathBuf::from("/etc/wwps/aegis"));
    }

    #[serial]
    #[test]
    fn load_and_validate_fails_when_config_exists_but_key_missing() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains(".key") || err.contains("不存在") || err.contains("Config file miss"),
            "error should mention missing key or config: {err}"
        );
    }

    #[serial]
    #[test]
    fn load_and_validate_fails_when_key_has_invalid_length() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join(".key"), [0u8; 16]).unwrap();
        fs::write(config_dir.join("config.enc"), b"{}").unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
        let err = format!("{:?}", result.err().unwrap());
        assert!(
            err.contains("Invalid key length") || err.contains(".key"),
            "error should mention invalid key length or .key: {err}"
        );
    }

    #[serial]
    #[test]
    fn load_and_validate_fails_when_config_missing_and_key_exists() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        fs::write(config_dir.join(".key"), [0u8; 32]).unwrap();

        let result = load_and_validate();
        assert!(result.is_err());
        let err = format!("{}", result.err().unwrap());
        assert!(
            err.contains("Config file miss") || err.contains("配置文件"),
            "error should mention missing config file: {err}"
        );
    }

    #[serial]
    #[test]
    fn load_and_validate_builds_working_totp_manager() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        // 同步构造 setup 等价产物：.key 由 SecurityManager 生成，config.enc 手工加密
        // token 需通过格式校验: <数字bot_id>:<token>
        // totp secret 用项目标准生成器（base64，兼容 validate_decrypted_config 与 TotpManager）
        let totp_secret = TotpManager::generate_new_secret();
        let security = SecurityManager::new(&config_dir.join(KEY_FILE)).unwrap();
        let encrypted = EncryptedConfig {
            token: Some(
                security
                    .encrypt(b"123456:ABCdefGHIjklMNOpqrsTUVwxyz")
                    .unwrap(),
            ),
            admin_id: Some(security.encrypt(b"42").unwrap()),
            totp_secret: Some(security.encrypt(totp_secret.as_bytes()).unwrap()),
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: Some("zh".to_string()),
            matrix_recovery_key: None,
        };
        fs::write(
            config_dir.join(CONFIG_FILE),
            serde_json::to_vec(&encrypted).unwrap(),
        )
        .unwrap();

        let (app_config, _security) = load_and_validate().unwrap();
        let manager = app_config.totp_manager.expect("totp_manager 应已构建");

        // TotpManager 功能完好：自身生成的当前码能通过 verify（证明密钥正确装载且可用）
        let code = manager.generate_current().unwrap();
        assert!(manager.verify(&code));
    }
}
