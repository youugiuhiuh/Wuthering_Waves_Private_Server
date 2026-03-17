use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::logic::security::SecurityManager;

pub const CONFIG_DIR: &str = "/etc/wwps/tgbot";

/// 配置目录；测试可通过环境变量 TGBOT_CONFIG_DIR 覆盖。
pub fn config_dir() -> PathBuf {
    std::env::var("TGBOT_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(CONFIG_DIR))
}
pub const KEY_FILE: &str = ".key";
pub const CONFIG_FILE: &str = "config.enc";
pub const BOT_SETTINGS_FILE: &str = "bot_settings.json";
pub const BOT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 10 * 60;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EncryptedConfig {
    pub token: Vec<u8>,
    pub admin_id: Vec<u8>,
    pub totp_secret: Vec<u8>,
    #[serde(default)]
    pub self_destruct_key_hash: Option<String>,
}

#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct SetupInput {
    token: String,
    admin_id: String,
    totp_secret: String,
}

impl Drop for EncryptedConfig {
    fn drop(&mut self) {
        self.token.zeroize();
        self.admin_id.zeroize();
        self.totp_secret.zeroize();
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
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<BotSettings>(&data) {
                    return s;
                }
            }
        }
        BotSettings {
            session_timeout_secs: DEFAULT_SESSION_TIMEOUT_SECS,
        }
    }

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

const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
const WWPS_CORE_BIN: &str = "/etc/wwps/wwps-core/wwps-core";

/// 同步执行 wwps-core/xray mldsa65，解析 Seed/Verify 并写入文件。供 setup 时调用（无 tokio）。
pub fn generate_reality_pq_keys_sync() -> Result<()> {
    let output = Command::new(WWPS_CORE_BIN)
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
    let dir = PathBuf::from("/etc/wwps");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("创建 /etc/wwps 失败")?;
    }
    fs::write(PQ_SEED_PATH, seed.as_bytes()).context("写入 reality_pq.seed 失败")?;
    fs::write(PQ_PUB_PATH, verify.as_bytes()).context("写入 reality_pq.pub 失败")?;
    Ok(())
}

/// 在管理初始化（tgbot --setup）时如无现有 PQ 配置则调用 mldsa65 生成。
fn sync_reality_pq_pub_on_setup() {
    if PathBuf::from(PQ_SEED_PATH).exists() || PathBuf::from(PQ_PUB_PATH).exists() {
        return;
    }
    if let Err(e) = generate_reality_pq_keys_sync() {
        log::error!("❌ Reality PQ 初始化: {}", e);
    }
}

pub async fn run_setup(token: &str, admin_id: &str, totp_secret: &str) -> Result<()> {
    let token = token.trim();
    let admin_id = admin_id.trim();
    let totp_secret = totp_secret.trim();
    let config_dir = config_dir();
    fs::create_dir_all(&config_dir)?;
    let security = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let encrypted_config = EncryptedConfig {
        token: security.encrypt(token.as_bytes())?,
        admin_id: security.encrypt(admin_id.as_bytes())?,
        totp_secret: security.encrypt(totp_secret.as_bytes())?,
        self_destruct_key_hash: None,
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

    let input: SetupInput = serde_json::from_str(&payload).context("解析 stdin 配置失败")?;
    run_setup(&input.token, &input.admin_id, &input.totp_secret).await
}

pub async fn verify_integrity() -> Result<()> {
    let config_dir = config_dir();
    if !config_dir.exists() {
        eprintln!(
            "❌ 配置文件目录不存在。请运行 `tgbot --setup <token> <admin_id> <totp_secret>` 进行初始化。"
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

pub fn harden_process() {
    #[cfg(target_os = "linux")]
    {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        let setrlimit_ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) };
        if setrlimit_ret != 0 {
            log::warn!("failed to disable core dumps via setrlimit");
        }

        let prctl_ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if prctl_ret != 0 {
            log::warn!("failed to mark process as non-dumpable");
        }
    }
}
