//! 集成测试：SecurityManager 加解密与 .key 一致性
//!
//! 验证加密/解密往返、密钥文件创建与读取。

use aegis::core::security::SecurityManager;
use secrecy::ExposeSecret;
use std::fs;

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

#[test]
fn encrypt_decrypt_roundtrip_varied_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(".key");
    let security = SecurityManager::new(&key_path).unwrap();

    let test_data: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"a".to_vec(),
        b"hello".to_vec(),
        b"x".repeat(16),
        b"x".repeat(255),
        b"x".repeat(4096),
    ];

    for data in &test_data {
        let encrypted = security.encrypt(data).unwrap();
        let decrypted = security.decrypt(&encrypted).unwrap();
        assert_eq!(
            decrypted.expose_secret().as_slice(),
            data.as_slice(),
            "roundtrip failed for data length {}",
            data.len()
        );
    }
}

#[test]
fn encrypt_produces_different_output_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(".key");
    let security = SecurityManager::new(&key_path).unwrap();
    let plain = b"deterministic input";

    let enc1 = security.encrypt(plain).unwrap();
    let enc2 = security.encrypt(plain).unwrap();

    assert_ne!(enc1, enc2);

    assert_eq!(
        security.decrypt(&enc1).unwrap().expose_secret().as_slice(),
        plain
    );
    assert_eq!(
        security.decrypt(&enc2).unwrap().expose_secret().as_slice(),
        plain
    );
}

#[test]
fn decrypt_tampered_ciphertext_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join(".key");
    let security = SecurityManager::new(&key_path).unwrap();
    let plain = b"tamper test";

    let mut encrypted = security.encrypt(plain).unwrap();
    if let Some(last) = encrypted.last_mut() {
        *last ^= 0xff;
    }
    let result = security.decrypt(&encrypted);
    assert!(result.is_err(), "decrypt should fail on tampered data");
}

#[test]
fn decrypt_wrong_key_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let security1 = SecurityManager::new(&dir.path().join(".key1")).unwrap();
    let security2 = SecurityManager::new(&dir.path().join(".key2")).unwrap();

    let encrypted = security1.encrypt(b"secret").unwrap();
    let result = security2.decrypt(&encrypted);
    assert!(result.is_err(), "decrypt with wrong key should fail");
}
