use crate::core::security::SecurityManager;
use anyhow::Result;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_CONFIG_DIR: &str = "/etc/wwps/aegis";
const STATE_SUBDIR: &str = "sni_state";
const KEY_FILE: &str = ".key";

/// 解析 SNI 状态目录。
///
/// 与 `bootstrap::config_dir()` 保持同一约定：`AEGIS_CONFIG_DIR` 优先，
/// 否则回落到生产默认值 `/etc/wwps/aegis`。
///
/// 修复前这里是硬编码的 `const SNI_STATE_DIR`，无视 `AEGIS_CONFIG_DIR`，
/// 导致任何跑状态逻辑的测试都会直接写生产目录（本轮排查期间已实际发生）。
/// 生产 systemd 单元不设该环境变量，故此改动不改变生产行为。
fn resolve_state_dir() -> PathBuf {
    match std::env::var("AEGIS_CONFIG_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => PathBuf::from(DEFAULT_CONFIG_DIR),
    }
    .join(STATE_SUBDIR)
}

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
        let state_dir = resolve_state_dir();
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
    use serial_test::serial;

    // 本模块用例均会改动 `AEGIS_CONFIG_DIR`（进程级全局环境变量），
    // 串行执行以免互相影响。nextest 每测试独立进程，天然隔离。

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

    // ---- T7: 状态目录必须遵循 AEGIS_CONFIG_DIR ----

    /// T7a: 设了 `AEGIS_CONFIG_DIR` 时，状态目录必须落在它下面。
    ///
    /// 修复前 `SNI_STATE_DIR` 是硬编码常量，任何测试跑状态逻辑都会写进
    /// `/etc/wwps/aegis/sni_state/` —— 本次排查期间已因此污染过生产目录一次。
    #[test]
    #[serial]
    fn t7_state_dir_follows_config_dir_env() {
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY: nextest 每测试独立进程；单进程并行时本模块用例均带 #[serial]。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", tmp.path()) };

        let p = SNIPersistence::new().expect("应能构造");
        assert_eq!(
            p.state_dir,
            tmp.path().join("sni_state"),
            "状态目录应位于 AEGIS_CONFIG_DIR/sni_state，实际为 {:?}",
            p.state_dir
        );
        assert!(p.state_dir.is_dir(), "状态目录应已创建");
        assert!(p.state_dir.join(".key").exists(), "密钥应与状态同目录");
    }

    /// T7b: 未设环境变量时回落生产默认值 —— 这是生产实际走的分支。
    #[test]
    #[serial]
    fn t7_state_dir_falls_back_to_default() {
        // SAFETY: 同上。必须先删干净，因为同进程可能残留。
        unsafe { std::env::remove_var("AEGIS_CONFIG_DIR") };

        let expected = PathBuf::from("/etc/wwps/aegis/sni_state");
        assert_eq!(resolve_state_dir(), expected);
    }
}
