//! 集成测试：setup 写入的 config 能被同一目录下的 SecurityManager 解密并成功建 TotpManager。
//!
//! 执行 `aegis --setup` 后读取 config.enc，解密 totp_secret 并 trim，用 TotpManager::new 断言成功。

use secrecy::{ExposeSecret, SecretString};
use std::fs;
use aegis::logic::security::SecurityManager;
use aegis::logic::totp::TotpManager;

#[derive(serde::Deserialize)]
struct EncryptedConfig {
    #[allow(dead_code)]
    token: Vec<u8>,
    #[allow(dead_code)]
    admin_id: Vec<u8>,
    totp_secret: Vec<u8>,
    #[serde(default)]
    _self_destruct_key_hash: Option<String>,
}

#[test]
fn setup_roundtrip_decrypt_and_totp_manager_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path();
    let bin = env!("CARGO_BIN_EXE_tgbot");
    let totp_secret = TotpManager::generate_new_secret();

    // 使用 aegis --setup 写入 config.enc + .key
    let out = std::process::Command::new(bin)
        .env("TGBOT_CONFIG_DIR", config_dir)
        .args(["--setup", "dummy_token", "123456", totp_secret.as_str()])
        .output()
        .expect("执行 aegis --setup 失败");

    assert!(
        out.status.success(),
        "setup 应成功。stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let key_path = config_dir.join(".key");
    let config_path = config_dir.join("config.enc");
    assert!(key_path.exists(), ".key 应存在");
    assert!(config_path.exists(), "config.enc 应存在");

    // 同一目录解密并建 TotpManager（与 main 一致：decrypt -> trim -> TotpManager::new）
    let security = SecurityManager::new(&key_path).expect("SecurityManager::new");
    let config_data = fs::read(&config_path).expect("读取 config.enc");
    let encrypted: EncryptedConfig = serde_json::from_slice(&config_data).expect("解析 config.enc");

    let totp_vec = security
        .decrypt(&encrypted.totp_secret)
        .expect("解密 totp_secret");
    let totp_secret_decrypted = String::from_utf8(totp_vec.expose_secret().to_vec()).expect("utf8");
    let totp_trimmed = totp_secret_decrypted.trim().to_string();

    let manager = TotpManager::new(&SecretString::from(totp_trimmed));
    assert!(manager.is_ok(), "解密并 trim 后应能建 TotpManager");
}
