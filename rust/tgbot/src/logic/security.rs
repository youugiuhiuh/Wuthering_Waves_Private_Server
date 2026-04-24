#![allow(dead_code, unused_variables)]
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::Result;
use libc::{mlock, munlock};
use obfstr::obfstr;
use rand::{RngCore, rngs::OsRng};
use secrecy::SecretVec;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use zeroize::Zeroizing;

pub struct SecurityManager {
    key: Zeroizing<[u8; 32]>,
}

const WIPE_CHUNK_SIZE: usize = 1024 * 1024;

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
            return Err(anyhow::anyhow!(obfstr!("Invalid key length").to_string()));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&key_data);

        Ok(Self {
            key: Zeroizing::new(key),
        })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| anyhow::anyhow!("Cipher init error: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("{}: {}", obfstr!("Encryption error"), e))?;

        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted_data: &[u8]) -> Result<SecretVec<u8>> {
        if encrypted_data.len() < 12 {
            return Err(anyhow::anyhow!(
                obfstr!("Invalid encrypted data length").to_string()
            ));
        }

        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .map_err(|e| anyhow::anyhow!("{}: {}", obfstr!("Cipher init error").to_string(), e))?;

        let nonce = Nonce::from_slice(&encrypted_data[..12]);
        let ciphertext = &encrypted_data[12..];

        let decrypted = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("{}: {}", obfstr!("Decryption error").to_string(), e))?;

        // Keep decrypted bytes zeroized on early-return paths, then move ownership out
        // without reallocating so mlock applies to the final backing allocation.
        let mut zeroizing_decrypted = Zeroizing::new(decrypted);
        let decrypted_vec = std::mem::take(&mut *zeroizing_decrypted);
        if !decrypted_vec.is_empty() {
            let ret = unsafe {
                mlock(
                    decrypted_vec.as_ptr() as *const libc::c_void,
                    decrypted_vec.len(),
                )
            };
            if ret != 0 {
                log::warn!("mlock decrypted data failed");
            }
        }

        let secret_vec = SecretVec::new(decrypted_vec);

        Ok(secret_vec)
    }
}

pub fn lock_memory(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }

    let ret = unsafe { mlock(data.as_ptr() as *const libc::c_void, data.len()) };
    if ret != 0 {
        log::warn!("mlock failed");
    }
}

pub fn unlock_memory(data: &mut [u8]) {
    if data.is_empty() {
        return;
    }

    let ret = unsafe { munlock(data.as_ptr() as *const libc::c_void, data.len()) };
    if ret != 0 {
        log::warn!("munlock failed");
    }
}

pub fn secure_wipe_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        fs::remove_file(path)?;
        return Ok(());
    }

    if file_type.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            secure_wipe_path(&entry.path())?;
        }
        fs::remove_dir(path)?;
    } else {
        let len = metadata.len();

        if len > 0 {
            let mut file = OpenOptions::new().write(true).open(path)?;
            // Chunked overwrite avoids allocating `len` bytes for large files.
            let zeros = vec![0u8; WIPE_CHUNK_SIZE];
            let mut remaining = len;
            while remaining > 0 {
                let write_len = std::cmp::min(remaining as usize, WIPE_CHUNK_SIZE);
                file.write_all(&zeros[..write_len])?;
                remaining -= write_len as u64;
            }
            file.sync_all()?;
        }
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_security_new_creates_key_file() {
        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("key");

        let result = SecurityManager::new(&key_path);

        assert!(result.is_ok());
        assert!(key_path.exists());
    }

    #[test]
    fn test_security_new_loads_existing_key() {
        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("key");

        // Create first manager
        let _sm1 = SecurityManager::new(&key_path).unwrap();

        // Create second manager with same key
        let sm2 = SecurityManager::new(&key_path);

        assert!(sm2.is_ok());
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        let plaintext = b"test data";
        let encrypted = sm.encrypt(plaintext).unwrap();
        let decrypted = sm.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted.expose_secret(), plaintext);
    }

    #[test]
    fn test_encrypt_empty_data() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        let encrypted = sm.encrypt(b"").unwrap();

        // Encrypted data should at least contain nonce (12 bytes)
        assert!(encrypted.len() >= 12);
    }

    #[test]
    fn test_decrypt_invalid_length() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        // Data too short (< 12 bytes for nonce)
        let result = sm.decrypt(&[0u8; 11]);

        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_corrupt_data() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        let mut encrypted = sm.encrypt(b"test").unwrap();
        // Corrupt the ciphertext (after nonce)
        if encrypted.len() > 15 {
            encrypted[15] ^= 0xFF;
        }

        let result = sm.decrypt(&encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_secure_wipe_path_file() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.txt");

        fs::write(&file_path, b"secret data").unwrap();
        assert!(file_path.exists());

        let result = secure_wipe_path(&file_path);

        assert!(result.is_ok());
        assert!(!file_path.exists());
    }

    #[test]
    fn test_secure_wipe_path_directory() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("test_dir");

        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("file1.txt"), b"data1").unwrap();
        fs::write(dir.join("file2.txt"), b"data2").unwrap();

        assert!(dir.exists());

        let result = secure_wipe_path(&dir);

        assert!(result.is_ok());
        assert!(!dir.exists());
    }

    #[test]
    fn test_secure_wipe_path_nonexistent() {
        let result = secure_wipe_path(Path::new("/nonexistent/path"));

        assert!(result.is_ok());
    }

    #[test]
    fn test_secure_wipe_path_empty_file() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("empty.txt");

        fs::write(&file_path, b"").unwrap();

        let result = secure_wipe_path(&file_path);

        assert!(result.is_ok());
        assert!(!file_path.exists());
    }

    #[test]
    fn test_lock_memory() {
        let mut data = vec![0u8; 1024];
        lock_memory(&mut data);
        // Should not panic
    }

    #[test]
    fn test_unlock_memory() {
        let mut data = vec![0u8; 1024];
        lock_memory(&mut data);
        unlock_memory(&mut data);
        // Should not panic
    }

    #[test]
    fn test_lock_memory_empty() {
        let mut data = vec![];
        lock_memory(&mut data);
        // Should not panic on empty slice
    }
}
