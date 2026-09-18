use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use aegis::core::i18n;
use aegis::core::paths::xray::{BIN, PQ_PUB_PATH, PQ_SEED_PATH};
use aegis::core::security::SecurityManager;

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
    #[serde(default)]
    pub token: Option<Vec<u8>>,
    #[serde(default)]
    pub admin_id: Option<Vec<u8>>,
    #[serde(default)]
    pub totp_secret: Option<Vec<u8>>,
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
    pub lang: Option<String>,
    #[serde(default)]
    pub matrix_recovery_key: Option<Vec<u8>>,
    #[serde(default)]
    pub simplex_port: Option<Vec<u8>>,
    #[serde(default)]
    pub simplex_admin_id: Option<Vec<u8>>,
}

#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct SetupInput {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    admin_id: Option<String>,
    #[serde(default)]
    totp_secret: Option<String>,
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
    matrix_recovery_key: Option<String>,
    #[serde(default)]
    simplex_port: Option<String>,
    #[serde(default)]
    simplex_admin_id: Option<String>,
}

impl Drop for EncryptedConfig {
    fn drop(&mut self) {
        if let Some(v) = &mut self.token {
            v.zeroize();
        }
        if let Some(v) = &mut self.admin_id {
            v.zeroize();
        }
        if let Some(v) = &mut self.totp_secret {
            v.zeroize();
        }
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
        if let Some(v) = &mut self.matrix_recovery_key {
            v.zeroize();
        }
        if let Some(v) = &mut self.simplex_port {
            v.zeroize();
        }
        if let Some(v) = &mut self.simplex_admin_id {
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
        fs::create_dir_all(&dir)?;
        fs::write(
            dir.join(BOT_SETTINGS_FILE),
            serde_json::to_string_pretty(self)?,
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
        .context(
            "执行 wwps-core/xray (Xray-core) mldsa65 失败（请确保已安装 wwps-core 或 xray）",
        )?;
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
    let dir = PathBuf::from("/etc/wwps");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("创建 /etc/wwps 失败")?;
    }
    fs::write(PQ_SEED_PATH, seed.as_bytes()).context("写入 reality_pq.seed 失败")?;
    fs::write(PQ_PUB_PATH, verify.as_bytes()).context("写入 reality_pq.pub 失败")?;
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
    token: Option<&str>,
    admin_id: Option<&str>,
    totp_secret: Option<&str>,
    matrix: Option<MatrixSetupConfig>,
    matrix_recovery_key: Option<&str>,
    simplex_port: Option<&str>,
    simplex_admin_id: Option<&str>,
) -> Result<()> {
    let config_dir = config_dir();
    fs::create_dir_all(&config_dir)?;
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

    let matrix_recovery_key = matrix_recovery_key
        .map(|k| security.encrypt(k.as_bytes()))
        .transpose()?;

    let simplex_port = simplex_port
        .map(|v| security.encrypt(v.trim().as_bytes()))
        .transpose()?;
    let simplex_admin_id = simplex_admin_id
        .map(|v| security.encrypt(v.trim().as_bytes()))
        .transpose()?;

    let encrypted_config = EncryptedConfig {
        token: token
            .map(|t| security.encrypt(t.trim().as_bytes()))
            .transpose()?,
        admin_id: admin_id
            .map(|t| security.encrypt(t.trim().as_bytes()))
            .transpose()?,
        totp_secret: totp_secret
            .map(|t| security.encrypt(t.trim().as_bytes()))
            .transpose()?,
        self_destruct_key_hash: None,
        matrix_homeserver,
        matrix_username,
        matrix_password,
        matrix_room_id,
        matrix_store_passphrase,
        lang: None,
        matrix_recovery_key,
        simplex_port,
        simplex_admin_id,
    };
    fs::write(
        config_dir.join(CONFIG_FILE),
        serde_json::to_vec(&encrypted_config)?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            config_dir.join(CONFIG_FILE),
            fs::Permissions::from_mode(0o600),
        )?;
    }
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

    let matrix_recovery_key = input.matrix_recovery_key.as_deref();
    let simplex_port = input.simplex_port.as_deref();
    let simplex_admin_id = input.simplex_admin_id.as_deref();

    run_setup(
        input.token.as_deref(),
        input.admin_id.as_deref(),
        input.totp_secret.as_deref(),
        matrix,
        matrix_recovery_key,
        simplex_port,
        simplex_admin_id,
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

/// Persist the chosen language to the encrypted config file.
#[allow(dead_code)]
pub fn save_lang_to_config(lang: i18n::Lang) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.lang = Some(lang.as_str().to_string());
    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}

/// Persist the self-destruct key hash to the encrypted config file.
#[allow(dead_code)]
pub fn save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.self_destruct_key_hash = hash;
    fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}

/// Atomically clear matrix_recovery_key from the encrypted config file.
/// Writes to a tmp file, fsyncs, then renames — same-filesystem atomic.
pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::Write;

    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig = serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    enc.matrix_recovery_key = None;
    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;

    let tmp_path = config_path.with_extension("enc.tmp");
    {
        let mut f = File::create(&tmp_path).context("创建临时文件失败")?;
        f.write_all(&new_data).context("写入临时文件失败")?;
        f.sync_all().context("fsync 临时文件失败")?;
    }
    fs::rename(&tmp_path, &config_path).context("rename config.enc 失败")?;
    println!("✅ 恢复密钥已从配置中清除（用完即焚）");
    Ok(())
}

/// 就地更新配置中的 `simplex_admin_id`，保留其余字段（含各自的密文）不变。
///
/// 与 `run_setup` 的区别：`run_setup` 从参数从零构造 `EncryptedConfig`，未传字段一律写
/// `None`；本函数以磁盘上的现有配置为底做定点替换，因此不会清空 `simplex_port` /
/// `totp_secret` / `matrix_*`，也不会轮换 TOTP。
///
/// 原子写（tmp + fsync + rename），权限显式钉 0600 —— 不能沿用
/// `clear_matrix_recovery_key` 里 `File::create` 落成 0644 的写法。
///
/// bin target 里 `pub` 不对外可见，Task 4（`--set-simplex-admin` CLI 接线）落地前
/// 在非测试构建中属于死代码；届时删掉这个 allow。
#[allow(dead_code)]
pub fn set_simplex_admin_id(config_dir: &Path, admin_id: i64) -> Result<()> {
    use std::io::Write;

    if admin_id <= 0 {
        anyhow::bail!("contactId 必须是正整数，收到 {admin_id}");
    }

    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig = serde_json::from_slice(&data).context("解析 config.enc 失败")?;

    let security = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    enc.simplex_admin_id = Some(security.encrypt(admin_id.to_string().as_bytes())?);

    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;
    let tmp_path = config_path.with_extension("enc.tmp");
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .context("创建临时文件失败")?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("设置临时文件权限失败")?;
        f.write_all(&new_data).context("写入临时文件失败")?;
        f.sync_all().context("fsync 临时文件失败")?;
    }
    fs::rename(&tmp_path, &config_path).context("rename config.enc 失败")?;

    println!("✅ SimpleX 管理员 contactId 已写入配置: {admin_id}");
    Ok(())
}

/// 安装 rustls 的进程级 crypto provider。
///
/// matrix-sdk 0.19 的 HTTP 栈是 reqwest 0.13 的 `rustls-no-provider`（aegis 用
/// `default-features = false` 关掉了 matrix-sdk 的 `rustls-aws-lc-rs` 默认特性），
/// reqwest 构建 Client 时读 `CryptoProvider::get_default()`，读不到会直接 panic：
/// "No rustls crypto provider is configured"（v1.5.9 线上启动即 abort）。
/// 启动阶段装一次 ring provider；已装过时返回 Err，忽略即可。
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
        token: Option<&str>,
        admin_id: Option<i64>,
        totp_secret: Option<&str>,
        self_destruct_key_hash: &Option<String>,
    ) -> Result<(), String> {
        if let Some(t) = token {
            self.validate_token(t)?;
        }
        if let Some(a) = admin_id {
            self.validate_admin_id(a)?;
        }
        if let Some(t) = totp_secret {
            self.validate_totp_secret(t)?;
        }
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
            Some("123456:ABCdefGHIjklMNOpqrsTUVwxyz"),
            Some(123456789),
            Some("JBSWY3DPEHPK3PXP"),
            &None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_decrypted_config_accepts_valid_config_with_hash() {
        let validator = ConfigValidator::new();
        let result = validator.validate_decrypted_config(
            Some("123456:ABCdefGHIjklMNOpqrsTUVwxyz"),
            Some(123456789),
            Some("JBSWY3DPEHPK3PXP"),
            &Some("a".repeat(64)),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_decrypted_config_accepts_empty_config() {
        let validator = ConfigValidator::new();
        let result = validator.validate_decrypted_config(None, None, None, &None);
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
    use secrecy::ExposeSecret;

    #[test]
    fn save_self_destruct_hash_compiles() {
        let _sig: fn(Option<String>) -> Result<()> = save_self_destruct_key_hash_to_config;
    }

    #[test]
    fn simplex_config_fields_round_trip() {
        let config = EncryptedConfig {
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
            simplex_port: Some(b"5225".to_vec()),
            simplex_admin_id: Some(b"42".to_vec()),
        };
        let json = serde_json::to_vec(&config).unwrap();
        let back: EncryptedConfig = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.simplex_port, Some(b"5225".to_vec()));
        assert_eq!(back.simplex_admin_id, Some(b"42".to_vec()));
    }

    /// 造一份填满所有字段的配置，用于验证定点替换不会碰到别的字段。
    fn seed_full_config(dir: &Path) -> EncryptedConfig {
        let seeded = EncryptedConfig {
            token: Some(b"123456:AA".to_vec()),
            admin_id: Some(b"777".to_vec()),
            totp_secret: Some(b"JBSWY3DPEHPK3PXP".to_vec()),
            self_destruct_key_hash: Some("a".repeat(64)),
            matrix_homeserver: Some(b"https://m.example".to_vec()),
            matrix_username: Some(b"@a:m.example".to_vec()),
            matrix_password: Some(b"pw".to_vec()),
            matrix_room_id: Some(b"!r:m.example".to_vec()),
            matrix_store_passphrase: Some(b"sp".to_vec()),
            lang: Some("zh".to_string()),
            matrix_recovery_key: Some(b"rk".to_vec()),
            simplex_port: Some(b"5225".to_vec()),
            simplex_admin_id: None,
        };
        fs::write(dir.join(CONFIG_FILE), serde_json::to_vec(&seeded).unwrap()).unwrap();
        seeded
    }

    fn read_config(dir: &Path) -> EncryptedConfig {
        serde_json::from_slice(&fs::read(dir.join(CONFIG_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn set_simplex_admin_id_preserves_every_other_field() {
        let dir = tempfile::TempDir::new().unwrap();
        let before = seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        let after = read_config(dir.path());
        assert_eq!(after.token, before.token);
        assert_eq!(after.admin_id, before.admin_id);
        assert_eq!(after.totp_secret, before.totp_secret, "TOTP 不得被轮换");
        assert_eq!(after.self_destruct_key_hash, before.self_destruct_key_hash);
        assert_eq!(after.matrix_homeserver, before.matrix_homeserver);
        assert_eq!(after.matrix_username, before.matrix_username);
        assert_eq!(after.matrix_password, before.matrix_password);
        assert_eq!(after.matrix_room_id, before.matrix_room_id);
        assert_eq!(
            after.matrix_store_passphrase,
            before.matrix_store_passphrase
        );
        assert_eq!(after.lang, before.lang);
        assert_eq!(after.matrix_recovery_key, before.matrix_recovery_key);
        assert_eq!(
            after.simplex_port, before.simplex_port,
            "simplex_port 不得被清空"
        );
        assert!(after.simplex_admin_id.is_some(), "新值必须被写入");
    }

    #[test]
    fn set_simplex_admin_id_stores_decryptable_value() {
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        let security = SecurityManager::new(&dir.path().join(KEY_FILE)).unwrap();
        let raw = read_config(dir.path()).simplex_admin_id.clone().unwrap();
        let plain = security.decrypt(&raw).unwrap();
        assert_eq!(
            String::from_utf8(plain.expose_secret().to_vec()).unwrap(),
            "42"
        );
    }

    #[test]
    fn set_simplex_admin_id_rejects_non_positive() {
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());
        assert!(set_simplex_admin_id(dir.path(), 0).is_err());
        assert!(set_simplex_admin_id(dir.path(), -1).is_err());
    }

    #[test]
    fn set_simplex_admin_id_errors_when_config_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(set_simplex_admin_id(dir.path(), 42).is_err());
    }

    #[test]
    fn set_simplex_admin_id_leaves_no_tmp_and_keeps_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        assert!(!dir.path().join("config.enc.tmp").exists());
        let mode = std::fs::metadata(dir.path().join(CONFIG_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
