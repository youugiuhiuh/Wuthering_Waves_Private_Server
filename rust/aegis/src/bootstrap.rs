use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use std::sync::{LazyLock, Mutex};

use aegis::core::i18n;
use aegis::core::paths::xray::{BIN, PQ_PUB_PATH, PQ_SEED_PATH};
use aegis::core::security::SecurityManager;
use aegis::core::security::secure_fs::{
    atomic_write_at, atomic_write_sensitive, open_dir, open_private_dir,
};

pub const CONFIG_DIR: &str = "/etc/wwps/aegis";

/// 配置目录；测试可通过环境变量 AEGIS_CONFIG_DIR 覆盖。
pub fn config_dir() -> PathBuf {
    std::env::var("AEGIS_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(CONFIG_DIR))
}
pub const KEY_FILE: &str = ".key";
pub const CONFIG_FILE: &str = "config.enc";
pub const BOT_SETTINGS_FILE: &str = "bot_settings.json";
#[allow(dead_code)]
pub const BOT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 10 * 60;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EncryptedConfig {
    pub token: Vec<u8>,
    pub admin_id: Vec<u8>,
    pub totp_secret: Vec<u8>,
    #[serde(default)]
    pub self_destruct_key_hash: Option<String>,
    #[serde(default)]
    pub matrix_homeserver: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_username: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_password: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_room_id: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_store_passphrase: Option<Vec<u8>>,
    #[serde(default)]
    pub discord_token: Option<Vec<u8>>,
    #[serde(default)]
    pub discord_admin_id: Option<Vec<u8>>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub matrix_recovery_key: Option<Vec<u8>>,
}

#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct SetupInput {
    token: String,
    admin_id: String,
    totp_secret: String,
    #[serde(default)]
    matrix_homeserver: Option<String>,
    #[serde(default)]
    matrix_username: Option<String>,
    #[serde(default)]
    matrix_password: Option<String>,
    #[serde(default)]
    matrix_room_id: Option<String>,
    #[serde(default)]
    matrix_store_passphrase: Option<String>,
    #[serde(default)]
    discord_token: Option<String>,
    #[serde(default)]
    discord_admin_id: Option<String>,
    #[serde(default)]
    matrix_recovery_key: Option<String>,
}

impl Drop for EncryptedConfig {
    fn drop(&mut self) {
        self.token.zeroize();
        self.admin_id.zeroize();
        self.totp_secret.zeroize();
        if let Some(v) = &mut self.matrix_homeserver {
            v.zeroize();
        }
        if let Some(v) = &mut self.matrix_username {
            v.zeroize();
        }
        if let Some(v) = &mut self.matrix_password {
            v.zeroize();
        }
        if let Some(v) = &mut self.matrix_room_id {
            v.zeroize();
        }
        if let Some(v) = &mut self.matrix_store_passphrase {
            v.zeroize();
        }
        if let Some(v) = &mut self.discord_token {
            v.zeroize();
        }
        if let Some(v) = &mut self.discord_admin_id {
            v.zeroize();
        }
        if let Some(v) = &mut self.matrix_recovery_key {
            v.zeroize();
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct BotSettings {
    #[serde(default = "BotSettings::default_session_timeout")]
    pub session_timeout_secs: u64,
}

impl BotSettings {
    fn default_session_timeout() -> u64 {
        DEFAULT_SESSION_TIMEOUT_SECS
    }

    pub fn load() -> Self {
        let path = config_dir().join(BOT_SETTINGS_FILE);
        if path.exists()
            && let Ok(data) = fs::read_to_string(&path)
            && let Ok(s) = serde_json::from_str::<BotSettings>(&data)
        {
            return s;
        }
        BotSettings {
            session_timeout_secs: DEFAULT_SESSION_TIMEOUT_SECS,
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        open_private_dir(&dir)?;
        atomic_write_sensitive(
            &dir.join(BOT_SETTINGS_FILE),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )?;
        Ok(())
    }
}

/// 同步执行 wwps-core/xray，解析 Seed/Verify 并写入文件。供 setup 时调用（无 tokio）。
pub fn generate_reality_pq_keys_sync() -> Result<()> {
    let output = Command::new(BIN)
        .arg("mldsa65")
        .output()
        .or_else(|_| Command::new("xray").arg("mldsa65").output())
        .context("执行 wwps-core/xray mldsa65 失败（请确保已安装 wwps-core 或 xray）")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("mldsa65 执行失败: {}", stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let seed = stdout
        .lines()
        .find(|l| l.starts_with("Seed:"))
        .and_then(|l| l.strip_prefix("Seed:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("mldsa65 输出未包含 Seed"))?;
    let verify = stdout
        .lines()
        .find(|l| l.starts_with("Verify:"))
        .and_then(|l| l.strip_prefix("Verify:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("mldsa65 输出未包含 Verify"))?;
    let dir = open_dir(Path::new("/etc/wwps")).context("打开 /etc/wwps 失败")?;
    atomic_write_at(&dir, OsStr::new("reality_pq.seed"), seed.as_bytes())
        .context("写入 reality_pq.seed 失败")?;
    atomic_write_at(&dir, OsStr::new("reality_pq.pub"), verify.as_bytes())
        .context("写入 reality_pq.pub 失败")?;
    Ok(())
}

/// 在管理初始化（aegis --setup）时如无现有 PQ 配置则调用 mldsa65 生成。
fn sync_reality_pq_pub_on_setup() {
    if PathBuf::from(PQ_SEED_PATH).exists() || PathBuf::from(PQ_PUB_PATH).exists() {
        return;
    }
    if let Err(e) = generate_reality_pq_keys_sync() {
        log::error!("❌ Reality PQ 初始化: {}", e);
    }
}

pub struct MatrixSetupConfig {
    homeserver: String,
    username: String,
    password: String,
    room_id: String,
    store_passphrase: String,
}

pub async fn run_setup(
    token: &str,
    admin_id: &str,
    totp_secret: &str,
    matrix: Option<MatrixSetupConfig>,
    discord_token: Option<&str>,
    discord_admin_id: Option<&str>,
    matrix_recovery_key: Option<&str>,
) -> Result<()> {
    let token = token.trim();
    let admin_id = admin_id.trim();
    let totp_secret = totp_secret.trim();
    let config_dir = config_dir();
    open_private_dir(&config_dir)?;
    let security = SecurityManager::new(&config_dir.join(KEY_FILE))?;

    let (
        matrix_homeserver,
        matrix_username,
        matrix_password,
        matrix_room_id,
        matrix_store_passphrase,
    ) = if let Some(m) = matrix {
        (
            Some(security.encrypt(m.homeserver.as_bytes())?),
            Some(security.encrypt(m.username.as_bytes())?),
            Some(security.encrypt(m.password.as_bytes())?),
            Some(security.encrypt(m.room_id.as_bytes())?),
            Some(security.encrypt(m.store_passphrase.as_bytes())?),
        )
    } else {
        (None, None, None, None, None)
    };

    let discord_token = discord_token
        .map(|t| security.encrypt(t.as_bytes()))
        .transpose()?;
    let discord_admin_id = discord_admin_id
        .map(|t| security.encrypt(t.as_bytes()))
        .transpose()?;

    let matrix_recovery_key = matrix_recovery_key
        .map(|k| security.encrypt(k.as_bytes()))
        .transpose()?;

    let encrypted_config = EncryptedConfig {
        token: security.encrypt(token.as_bytes())?,
        admin_id: security.encrypt(admin_id.as_bytes())?,
        totp_secret: security.encrypt(totp_secret.as_bytes())?,
        self_destruct_key_hash: None,
        matrix_homeserver,
        matrix_username,
        matrix_password,
        matrix_room_id,
        matrix_store_passphrase,
        discord_token,
        discord_admin_id,
        lang: None,
        matrix_recovery_key,
    };
    atomic_write_sensitive(
        &config_dir.join(CONFIG_FILE),
        &serde_json::to_vec(&encrypted_config)?,
    )?;
    // 管理初始化时同步 PQ 公钥到默认路径（若通过环境变量提供且尚未写入）。
    sync_reality_pq_pub_on_setup();
    println!("✅ Setup completed successfully.");
    Ok(())
}

pub async fn run_setup_from_stdin() -> Result<()> {
    let mut payload = Zeroizing::new(String::new());
    std::io::stdin()
        .read_to_string(&mut payload)
        .context("读取 stdin 配置失败")?;

    let mut input: SetupInput = serde_json::from_str(&payload).context("解析 stdin 配置失败")?;

    let matrix = {
        let hs = input.matrix_homeserver.take();
        let un = input.matrix_username.take();
        let pw = input.matrix_password.take();
        let rid = input.matrix_room_id.take();
        let sp = input.matrix_store_passphrase.take();
        match (hs, un, pw, rid, sp) {
            (
                Some(homeserver),
                Some(username),
                Some(password),
                Some(room_id),
                Some(store_passphrase),
            ) => Some(MatrixSetupConfig {
                homeserver,
                username,
                password,
                room_id,
                store_passphrase,
            }),
            _ => None,
        }
    };

    let discord_token = input.discord_token.as_deref();
    let discord_admin_id = input.discord_admin_id.as_deref();
    let matrix_recovery_key = input.matrix_recovery_key.as_deref();

    run_setup(
        &input.token,
        &input.admin_id,
        &input.totp_secret,
        matrix,
        discord_token,
        discord_admin_id,
        matrix_recovery_key,
    )
    .await
}

pub async fn verify_integrity() -> Result<()> {
    let config_dir = config_dir();
    if !config_dir.exists() {
        eprintln!(
            "❌ 配置文件目录不存在。请运行 `aegis --setup <token> <admin_id> <totp_secret>` 进行初始化。"
        );
        std::process::exit(1);
    }

    let current_exe = std::env::current_exe().context("Failed to get current executable path")?;
    let content = fs::read(&current_exe).context("Failed to read executable")?;

    let mut hasher = Sha256::new();
    hasher.update(&content);
    let hash = hex::encode(hasher.finalize());

    eprintln!("Binary Integrity Hash: {}", hash);
    Ok(())
}

static CONFIG_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Apply a mutation to the encrypted config file under a lock.
/// The mutation function receives `&mut EncryptedConfig` and should modify it.
/// On success the complete value is validated and atomically written.
fn update_config<F>(f: F) -> Result<()>
where
    F: FnOnce(&mut EncryptedConfig) -> Result<()>,
{
    let _lock = CONFIG_MUTEX.lock().unwrap();
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&path).context("读取 config.enc 失败")?;
    let mut config: EncryptedConfig =
        serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    f(&mut config)?;
    atomic_write_sensitive(
        &path,
        &serde_json::to_vec(&config).context("序列化 config.enc 失败")?,
    )?;
    Ok(())
}

/// Persist the chosen language to the encrypted config file.
#[allow(dead_code)]
pub fn save_lang_to_config(lang: i18n::Lang) -> Result<()> {
    update_config(|config| {
        config.lang = Some(lang.as_str().to_string());
        Ok(())
    })
}

/// Persist the self-destruct key hash to the encrypted config file.
#[allow(dead_code)]
pub fn save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()> {
    update_config(|config| {
        config.self_destruct_key_hash = hash;
        Ok(())
    })
}

/// Atomically clear matrix_recovery_key from the encrypted config file.
pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()> {
    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig = serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    enc.matrix_recovery_key = None;
    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;
    atomic_write_sensitive(&config_path, &new_data)?;
    println!("✅ 恢复密钥已从配置中清除（用完即焚）");
    Ok(())
}

pub fn harden_process() {
    #[cfg(target_os = "linux")]
    {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        // SAFETY:
        // - limit is a stack-initialized rlimit struct; the pointer is guaranteed aligned by the compiler
        // - setrlimit does not modify the memory that limit points to
        let setrlimit_ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) };
        if setrlimit_ret != 0 {
            log::warn!("failed to disable core dumps via setrlimit");
        }

        // SAFETY:
        // - prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) uses only integer arguments, no pointer dereference
        // - Only modifies a kernel-side process flag; accesses no userspace memory beyond the provided integer args
        let prctl_ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if prctl_ret != 0 {
            log::warn!("failed to mark process as non-dumpable");
        }
    }
}

pub struct ConfigValidator;

#[allow(clippy::new_without_default)]
impl ConfigValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_decrypted_config(
        &self,
        token: &str,
        admin_id: i64,
        totp_secret: &str,
        self_destruct_key_hash: &Option<String>,
    ) -> Result<(), String> {
        self.validate_token(token)?;
        self.validate_admin_id(admin_id)?;
        self.validate_totp_secret(totp_secret)?;
        if let Some(hash) = self_destruct_key_hash {
            self.validate_self_destruct_key_hash(hash)?;
        }
        Ok(())
    }

    fn validate_token(&self, token: &str) -> Result<(), String> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err("Token 不能为空".to_string());
        }
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "Token 格式无效: 应为 `<bot_id>:<token>` 格式，实际为 {}",
                trimmed
            ));
        }
        if parts[0].is_empty() || !parts[0].chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("Token 的 bot_id 部分无效: {}", parts[0]));
        }
        if parts[1].is_empty() {
            return Err("Token 的 token 部分不能为空".to_string());
        }
        Ok(())
    }

    fn validate_admin_id(&self, admin_id: i64) -> Result<(), String> {
        if admin_id <= 0 {
            return Err(format!("Admin ID 无效: {} (应大于 0)", admin_id));
        }
        Ok(())
    }

    fn validate_totp_secret(&self, totp_secret: &str) -> Result<(), String> {
        let trimmed = totp_secret.trim();
        if trimmed.is_empty() {
            return Err("TOTP Secret 不能为空".to_string());
        }
        let secret_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, trimmed)
                .map_err(|_| "TOTP Secret 不是有效的 base64 编码".to_string())?;
        if secret_bytes.len() < 10 {
            return Err("TOTP Secret 太短 (至少需要 10 字节)".to_string());
        }
        Ok(())
    }

    fn validate_self_destruct_key_hash(&self, hash: &str) -> Result<(), String> {
        let trimmed = hash.trim();
        if trimmed.len() != 64 {
            return Err(format!(
                "Self-destruct key hash 长度无效: {} (应为 64 字符的 SHA-256 十六进制)",
                trimmed.len()
            ));
        }
        if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Self-destruct key hash 包含无效的十六进制字符".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod config_validator_tests {
    use super::*;

    #[test]
    fn validate_decrypted_config_accepts_valid_config() {
        let validator = ConfigValidator::new();
        let result = validator.validate_decrypted_config(
            "123456:ABCdefGHIjklMNOpqrsTUVwxyz",
            123456789,
            "JBSWY3DPEHPK3PXP",
            &None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_decrypted_config_accepts_valid_config_with_hash() {
        let validator = ConfigValidator::new();
        let result = validator.validate_decrypted_config(
            "123456:ABCdefGHIjklMNOpqrsTUVwxyz",
            123456789,
            "JBSWY3DPEHPK3PXP",
            &Some("a".repeat(64)),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_token_rejects_empty_token() {
        let validator = ConfigValidator::new();
        let result = validator.validate_token("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Token 不能为空");
    }

    #[test]
    fn validate_token_rejects_whitespace_only_token() {
        let validator = ConfigValidator::new();
        let result = validator.validate_token("   \n\t  ");
        assert!(result.is_err());
    }

    #[test]
    fn validate_token_rejects_invalid_format() {
        let validator = ConfigValidator::new();
        assert!(validator.validate_token("not-valid-format").is_err());
        assert!(validator.validate_token("123456").is_err());
        assert!(validator.validate_token("123456:").is_err());
        assert!(validator.validate_token(":token").is_err());
    }

    #[test]
    fn validate_token_rejects_non_numeric_bot_id() {
        let validator = ConfigValidator::new();
        let result = validator.validate_token("abc:token");
        assert!(result.is_err());
    }

    #[test]
    fn validate_admin_id_rejects_zero() {
        let validator = ConfigValidator::new();
        let result = validator.validate_admin_id(0);
        assert!(result.is_err());
    }

    #[test]
    fn validate_admin_id_rejects_negative() {
        let validator = ConfigValidator::new();
        assert!(validator.validate_admin_id(-1).is_err());
        assert!(validator.validate_admin_id(-987654321).is_err());
    }

    #[test]
    fn validate_admin_id_accepts_positive() {
        let validator = ConfigValidator::new();
        assert!(validator.validate_admin_id(1).is_ok());
        assert!(validator.validate_admin_id(123456789).is_ok());
    }

    #[test]
    fn validate_totp_secret_rejects_empty_secret() {
        let validator = ConfigValidator::new();
        let result = validator.validate_totp_secret("");
        assert!(result.is_err());
    }

    #[test]
    fn validate_totp_secret_rejects_invalid_base64() {
        let validator = ConfigValidator::new();
        let result = validator.validate_totp_secret("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn validate_totp_secret_rejects_too_short_secret() {
        let validator = ConfigValidator::new();
        let result = validator.validate_totp_secret("aB");
        assert!(result.is_err());
    }

    #[test]
    fn validate_self_destruct_key_hash_rejects_wrong_length() {
        let validator = ConfigValidator::new();
        assert!(validator.validate_self_destruct_key_hash("abc123").is_err());
        assert!(
            validator
                .validate_self_destruct_key_hash(&"a".repeat(63))
                .is_err()
        );
        assert!(
            validator
                .validate_self_destruct_key_hash(&"a".repeat(65))
                .is_err()
        );
    }

    #[test]
    fn validate_self_destruct_key_hash_rejects_invalid_hex_chars() {
        let validator = ConfigValidator::new();
        let result = validator.validate_self_destruct_key_hash(&"g".repeat(64));
        assert!(result.is_err());
    }

    #[test]
    fn validate_self_destruct_key_hash_accepts_valid_sha256() {
        let validator = ConfigValidator::new();
        assert!(
            validator
                .validate_self_destruct_key_hash(&"a".repeat(64))
                .is_ok()
        );
        assert!(
            validator
                .validate_self_destruct_key_hash(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                )
                .is_ok()
        );
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_self_destruct_hash_compiles() {
        let _sig: fn(Option<String>) -> Result<()> = save_self_destruct_key_hash_to_config;
    }

    #[test]
    fn discord_config_fields_round_trip() {
        let config = EncryptedConfig {
            token: b"t".to_vec(),
            admin_id: b"a".to_vec(),
            totp_secret: b"s".to_vec(),
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            lang: None,
            discord_token: Some(b"dt".to_vec()),
            discord_admin_id: Some(b"da".to_vec()),
            matrix_recovery_key: None,
        };
        let json = serde_json::to_vec(&config).unwrap();
        let deserialized: EncryptedConfig = serde_json::from_slice(&json).unwrap();
        assert_eq!(deserialized.discord_token, Some(b"dt".to_vec()));
        assert_eq!(deserialized.discord_admin_id, Some(b"da".to_vec()));
    }

    #[test]
    fn concurrent_mutations_preserve_both_updates() {
        let dir = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
        }
        // atomic_write_sensitive requires 0o700 on the parent directory
        let _ = std::fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        );
        let config_dir = dir.path();
        let key = [0u8; 32];
        fs::write(config_dir.join(".key"), key).unwrap();
        let init = EncryptedConfig {
            token: b"t".to_vec(),
            admin_id: b"0".to_vec(),
            totp_secret: b"s".to_vec(),
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            lang: None,
            discord_token: None,
            discord_admin_id: None,
            matrix_recovery_key: None,
        };
        fs::write(
            config_dir.join("config.enc"),
            serde_json::to_vec(&init).unwrap(),
        )
        .unwrap();

        let t1 = std::thread::spawn(|| {
            save_lang_to_config(i18n::Lang::En).unwrap();
        });
        let t2 = std::thread::spawn(|| {
            save_self_destruct_key_hash_to_config(Some("a".repeat(64))).unwrap();
        });
        t1.join().unwrap();
        t2.join().unwrap();

        let data = fs::read(config_dir.join("config.enc")).unwrap();
        let config: EncryptedConfig = serde_json::from_slice(&data).unwrap();
        assert_eq!(config.lang, Some("en".to_string()));
        assert_eq!(config.self_destruct_key_hash, Some("a".repeat(64)));
    }
}
