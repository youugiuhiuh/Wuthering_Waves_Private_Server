//! 集成测试：TOTP 密钥含换行/空格时 trim 后能正常初始化
//!
//! 模拟安装器从 `tgbot --generate-totp-secret` 读取输出（带换行）后写入配置，
//! 启动时解密并 trim 再交给 TotpManager 应成功。

use secrecy::SecretString;
use tgbot::logic::totp::TotpManager;

#[test]
fn totp_secret_with_trailing_newline_fails_without_trim() {
    let secret = TotpManager::generate_new_secret();
    let with_newline = format!("{}\n", secret);
    let result = TotpManager::new(&SecretString::from(with_newline));
    assert!(result.is_err());
}

#[test]
fn totp_secret_trimmed_after_decrypt_succeeds() {
    let secret = TotpManager::generate_new_secret();
    let as_from_installer = format!("{}\n", secret);
    let trimmed = as_from_installer.trim();
    let result = TotpManager::new(&SecretString::from(trimmed.to_string()));
    assert!(result.is_ok());
}

#[test]
fn totp_secret_with_spaces_trimmed_succeeds() {
    let secret = TotpManager::generate_new_secret();
    let with_spaces = format!("  {}  \r\n ", secret);
    let trimmed = with_spaces.trim();
    let result = TotpManager::new(&SecretString::from(trimmed.to_string()));
    assert!(result.is_ok());
}
