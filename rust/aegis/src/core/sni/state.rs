use crate::core::security::SecurityManager;
use anyhow::Result;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const SNI_STATE_DIR: &str = "/etc/wwps/aegis/sni_state";
const KEY_FILE: &str = ".key";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SNIState {
    #[serde(rename = "d")]
    pub domains: Vec<String>,
    #[serde(rename = "s")]
    pub shuffled_indices: Vec<usize>,
    #[serde(rename = "u")]
    pub used_count: usize,
    #[serde(rename = "c")]
    pub created_at: String,
}

impl SNIState {
    pub fn new(domains: Vec<String>) -> Self {
        Self {
            domains,
            shuffled_indices: Vec::new(),
            used_count: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.shuffled_indices.is_empty() && !self.domains.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.shuffled_indices.len()
    }

    pub fn pop_index(&mut self) -> Option<usize> {
        let idx = self.shuffled_indices.pop()?;
        self.used_count += 1;
        Some(idx)
    }

    pub fn set_shuffled_indices(&mut self, indices: Vec<usize>) {
        self.shuffled_indices = indices;
    }

    pub fn reset(&mut self) {
        self.shuffled_indices.clear();
        self.used_count = 0;
        self.created_at = chrono::Utc::now().to_rfc3339();
    }
}

pub struct SNIPersistence {
    security: SecurityManager,
    state_dir: PathBuf,
}

impl SNIPersistence {
    pub fn new() -> Result<Self> {
        let state_dir = PathBuf::from(SNI_STATE_DIR);
        let key_path = state_dir.join(KEY_FILE);

        if !state_dir.exists() {
            fs::create_dir_all(&state_dir)?;
        }

        let security = SecurityManager::new(&key_path)?;

        Ok(Self {
            security,
            state_dir,
        })
    }

    pub fn get_state_path(&self, key: &str) -> PathBuf {
        self.state_dir.join(format!("{}.enc", key))
    }

    pub fn load(&self, key: &str) -> Option<SNIState> {
        let path = self.get_state_path(key);

        if !path.exists() {
            log::debug!("SNI state file not found: {}", path.display());
            return None;
        }

        let encrypted_data = match fs::read(&path) {
            Ok(data) => data,
            Err(e) => {
                log::warn!("Failed to read SNI state file {}: {}", path.display(), e);
                if let Err(rm_err) = fs::remove_file(&path) {
                    log::warn!("Failed to remove corrupted file: {}", rm_err);
                }
                return None;
            }
        };

        let decrypted = match self.security.decrypt(&encrypted_data) {
            Ok(data) => data,
            Err(e) => {
                log::warn!("Failed to decrypt SNI state {}: {}", key, e);
                if let Err(rm_err) = fs::remove_file(&path) {
                    log::warn!("Failed to remove corrupted file: {}", rm_err);
                }
                return None;
            }
        };

        let decrypted_vec: Vec<u8> = decrypted.expose_secret().clone();
        match serde_json::from_slice::<SNIState>(&decrypted_vec) {
            Ok(state) => {
                log::debug!(
                    "Loaded SNI state for {}: {} domains, remaining={}, total_used={}",
                    key,
                    state.domains.len(),
                    state.shuffled_indices.len(),
                    state.used_count
                );
                Some(state)
            }
            Err(e) => {
                log::warn!("Failed to parse SNI state {}: {}", key, e);
                if let Err(rm_err) = fs::remove_file(&path) {
                    log::warn!("Failed to remove corrupted file: {}", rm_err);
                }
                None
            }
        }
    }

    pub fn save(&self, key: &str, state: &SNIState) -> Result<()> {
        let path = self.get_state_path(key);
        let json_data = serde_json::to_vec(state)?;
        let encrypted_data = self.security.encrypt(&json_data)?;

        fs::write(&path, encrypted_data)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        log::debug!(
            "Saved SNI state for {}: {} domains, remaining={}, total_used={}",
            key,
            state.domains.len(),
            state.shuffled_indices.len(),
            state.used_count
        );

        Ok(())
    }

    pub fn reset(&self, key: &str) -> Result<()> {
        let path = self.get_state_path(key);
        if path.exists() {
            fs::remove_file(&path)?;
            log::info!("Reset SNI state for {}", key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sni_state_new() {
        let state = SNIState::new(vec!["a.com".to_string(), "b.com".to_string()]);
        assert_eq!(state.domains.len(), 2);
        assert!(state.shuffled_indices.is_empty());
        assert_eq!(state.used_count, 0);
    }

    #[test]
    fn test_sni_state_pop_index() {
        let mut state = SNIState::new(vec!["a.com".to_string(), "b.com".to_string()]);
        state.shuffled_indices = vec![1, 0];

        let idx = state.pop_index();
        assert_eq!(idx, Some(0)); // pop 从末尾取
        assert_eq!(state.used_count, 1);
        assert_eq!(state.shuffled_indices.len(), 1);

        let idx = state.pop_index();
        assert_eq!(idx, Some(1));
        assert_eq!(state.used_count, 2);
        assert_eq!(state.shuffled_indices.len(), 0);
    }

    #[test]
    fn test_sni_state_is_exhausted() {
        let mut state = SNIState::new(vec!["a.com".to_string(), "b.com".to_string()]);
        assert!(state.is_exhausted()); // indices 为空，需要初始化

        state.shuffled_indices = vec![0, 1];
        assert!(!state.is_exhausted());

        state.pop_index();
        state.pop_index();
        assert!(state.is_exhausted());
    }

    #[test]
    fn test_sni_state_serialization() {
        let mut state = SNIState::new(vec!["a.com".to_string(), "b.com".to_string()]);
        state.shuffled_indices = vec![1, 0];

        let json = serde_json::to_string(&state).unwrap();
        let parsed: SNIState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.domains, state.domains);
        assert_eq!(parsed.shuffled_indices, state.shuffled_indices);
        assert_eq!(parsed.used_count, state.used_count);
    }
}
