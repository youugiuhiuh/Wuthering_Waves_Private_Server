//! 集成测试：SecurityManager 加解密与 .key 一致性
//!
//! 验证加密/解密往返、密钥文件创建与读取。

use secrecy::ExposeSecret;
use std::fs;
use tgbot::logic::security::SecurityManager;

#[test]
fn security_manager_creates_key_and_encrypt_decrypt_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(".key");
    assert!(!key_path.exists());

    let security = SecurityManager::new(&key_path).unwrap();
    assert!(key_path.exists());
    assert_eq!(fs::read(&key_path).unwrap().len(), 32);

    let plain = b"totp_secret_with_newline\n";
    let encrypted = security.encrypt(plain).unwrap();
    assert!(encrypted.len() > 12);

    let decrypted = security.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted.expose_secret().as_slice(), plain);
}

#[test]
fn security_manager_same_key_decrypts_encrypted_data() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(".key");

    let security1 = SecurityManager::new(&key_path).unwrap();
    let encrypted = security1.encrypt(b"same_key").unwrap();
    drop(security1);

    let security2 = SecurityManager::new(&key_path).unwrap();
    let decrypted = security2.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted.expose_secret().as_slice(), b"same_key");
}
