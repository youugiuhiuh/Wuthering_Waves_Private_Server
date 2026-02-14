#![allow(dead_code, unused_variables)]
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::Result;
use libc::{mlock, munlock};
use rand::{RngCore, rngs::OsRng};
use secrecy::SecretString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use zeroize::Zeroizing;

pub struct SecurityManager {
    key: Zeroizing<[u8; 32]>,
}

impl SecurityManager {
    pub fn new(key_path: &Path) -> Result<Self> {
        if !key_path.exists() {
            let mut key = [0u8; 32];
            OsRng.fill_bytes(&mut key);
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(key_path, key)?;
            // Set restrictive permissions (root only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(key_path, fs::Permissions::from_mode(0o600))?;
            }
        }

        let key_data = fs::read(key_path)?;
        if key_data.len() != 32 {
            return Err(anyhow::anyhow!("Invalid key length"));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&key_data);

        Ok(Self {
            key: Zeroizing::new(key),
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| anyhow::anyhow!("Cipher init error: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("Encryption error: {}", e))?;

        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted_data: &[u8]) -> Result<SecretString> {
        if encrypted_data.len() < 12 {
            return Err(anyhow::anyhow!("Invalid encrypted data length"));
        }

        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| anyhow::anyhow!("Cipher init error: {}", e))?;

        let nonce = Nonce::from_slice(&encrypted_data[..12]);
        let ciphertext = &encrypted_data[12..];

        let decrypted = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Decryption error: {}", e))?;

        // mlock the decrypted data
        unsafe {
            mlock(decrypted.as_ptr() as *const libc::c_void, decrypted.len());
        }

        let secret_str = SecretString::from(String::from_utf8(decrypted)?);
        // Note: String::from_utf8 consumes the Vec, but secrecy will eventually zero it if it was original Vec.
        // But the String itself might be moved. Secrecy handles this.

        Ok(secret_str)
    }
}

pub fn lock_memory(data: &mut [u8]) {
    unsafe {
        mlock(data.as_ptr() as *const libc::c_void, data.len());
    }
}

pub fn unlock_memory(data: &mut [u8]) {
    unsafe {
        munlock(data.as_ptr() as *const libc::c_void, data.len());
    }
}

pub fn secure_wipe_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            secure_wipe_path(&entry.path())?;
        }
        fs::remove_dir(path)?;
    } else {
        let metadata = fs::metadata(path)?;
        let len = metadata.len();

        if len > 0 {
            let mut file = OpenOptions::new().write(true).open(path)?;
            // 写入随机数据覆盖 (简单实现：0x00)
            // 真正的安全擦除可能需要多次随机覆盖，考虑到 SSD 寿命和效率，这里做一次 0 填充
            let zeros = vec![0u8; len as usize];
            file.write_all(&zeros)?;
            file.sync_all()?;
        }
        fs::remove_file(path)?;
    }
    Ok(())
}
