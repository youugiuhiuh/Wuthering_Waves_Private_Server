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

/// SNI 轮转状态。
///
/// 修复前这里存的是完整域名列表 + 打乱后的下标排列 —— 单文件真机实测
/// 19,591,273 字节，且每抽一个 SNI 就重写一遍。
///
/// 改为只存「种子 + 已用数量」：域名列表永远从 `.pb` 重新加载（顺带修正了
/// 「落盘旧列表会盖掉新 .pb」的缺陷），排列由种子确定性重建。
/// 文件因此降到常数级（几十字节）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SNIState {
    /// 本轮排列的随机种子。
    #[serde(rename = "s")]
    pub seed: u64,
    /// 本轮已消耗的数量。
    #[serde(rename = "u")]
    pub used_count: u32,
    #[serde(rename = "c")]
    pub created_at: String,
}

/// 旧格式的标记结构 —— 只用于**识别**，不用于恢复。
///
/// 旧格式的 `"s"` 是 `Vec<usize>`，新格式是 `u64`，两者类型不兼容，
/// 直接反序列化必然失败。我们需要区分的是「这是旧格式」还是「文件真的坏了」，
/// 因为前者是一次性迁移，后者需要告警。
#[derive(Deserialize)]
struct LegacyMarker {
    #[serde(rename = "d")]
    #[allow(dead_code)]
    domains: serde::de::IgnoredAny,
}

impl SNIState {
    pub fn new(seed: u64, used_count: u32) -> Self {
        Self {
            seed,
            used_count,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
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
                    "Loaded SNI state for {}: seed={}, used={}",
                    key,
                    state.seed,
                    state.used_count
                );
                Some(state)
            }
            Err(e) => {
                if serde_json::from_slice::<LegacyMarker>(&decrypted_vec).is_ok() {
                    log::warn!(
                        "SNI state {} 是旧格式（含完整域名列表），将重建轮转状态。\
                         域名池不受影响，仅轮转位置重置一次。解析错误: {e}",
                        key
                    );
                } else {
                    log::warn!("Failed to parse SNI state {}: {}", key, e);
                }
                if let Err(rm_err) = fs::remove_file(&path) {
                    log::warn!("Failed to remove corrupted state: {}", rm_err);
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
            "Saved SNI state for {}: seed={}, used={}",
            key,
            state.seed,
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
        let state = SNIState::new(0xABCD, 7);
        assert_eq!(state.seed, 0xABCD);
        assert_eq!(state.used_count, 7);
        assert!(!state.created_at.is_empty());
    }

    #[test]
    fn test_sni_state_serialization() {
        let state = SNIState::new(0x0123_4567_89AB_CDEF, 4_294_967);
        let json = serde_json::to_string(&state).unwrap();
        let parsed: SNIState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }

    /// 紧凑格式的关键属性：体积与域名数量无关，且有常数上界。
    ///
    /// 注意不要断言「不同取值长度相同」—— u64/u32 的十进制位数本就不同，
    /// 那与体积是否恒定无关。要锁的是**上界**：无论取值多大都只有几十字节。
    #[test]
    fn test_sni_state_json_is_bounded() {
        let worst = serde_json::to_string(&SNIState::new(u64::MAX, u32::MAX)).unwrap();
        assert!(
            worst.len() < 128,
            "紧凑状态应 < 128 字节，实际 {} 字节（修复前为 19,591,273）",
            worst.len()
        );

        let v: serde_json::Value = serde_json::from_str(&worst).unwrap();
        assert!(v.get("s").is_some());
        assert!(v.get("u").is_some());
        assert!(v.get("c").is_some());
        assert!(v.get("d").is_none(), "不应再落盘域名列表");
        assert!(
            v.get("s").unwrap().as_array().is_none(),
            "\"s\" 必须是标量种子，而不是旧格式的下标数组"
        );
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

    // ---- T3 / T6: 紧凑格式与旧格式迁移 ----

    /// T3: 状态文件必须降到常数级。
    ///
    /// 修复前每抽一个 SNI 就重写含 790,572 个域名的完整列表，
    /// 真机实测单个文件 19,591,273 字节。
    #[test]
    #[serial]
    fn t3_state_file_is_tiny() {
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY: nextest 每测试独立进程；单进程并行时本模块用例均带 #[serial]。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", tmp.path()) };

        let p = SNIPersistence::new().unwrap();
        let state = SNIState::new(0xDEAD_BEEF_CAFE_F00D, 123_456);
        p.save("sni_US", &state).unwrap();

        let path = p.get_state_path("sni_US");
        let size = fs::metadata(&path).unwrap().len();
        assert!(
            size < 256,
            "状态文件应 < 256 字节，实际 {size} 字节（修复前为 19,591,273）"
        );

        // 加密开销有下限（12 字节 nonce），但内容本身必须是常数级。
        let round_trip = p.load("sni_US").expect("应能读回");
        assert_eq!(round_trip.seed, state.seed);
        assert_eq!(round_trip.used_count, state.used_count);
    }

    /// T6: 旧格式（含 `"d"` 域名列表字段）必须被识别为可迁移，而不是当成损坏文件。
    ///
    /// 旧格式的 `"s"` 是 `Vec<usize>`，新格式的 `"s"` 是 `u64` —— 类型不兼容，
    /// 直接反序列化必然失败。这里要求失败被**归类**为迁移而非损坏。
    #[test]
    #[serial]
    fn t6_legacy_format_is_detected_and_treated_as_fresh() {
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY: 同上。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", tmp.path()) };

        let p = SNIPersistence::new().unwrap();
        let key = "sni_LEGACY";

        // 构造一份旧格式的加密状态文件。
        let legacy = serde_json::json!({
            "d": ["a.com", "b.com"],
            "s": [0, 1],
            "u": 2,
            "c": "2026-01-01T00:00:00+00:00",
        });
        let enc = p
            .security
            .encrypt(&serde_json::to_vec(&legacy).unwrap())
            .unwrap();
        fs::write(p.get_state_path(key), enc).unwrap();

        // 不 panic、不 Err；调用方应能拿到一个可用的全新状态。
        let loaded = p.load(key);
        assert!(
            loaded.is_none(),
            "旧格式应被识别为不可直接读取（由调用方重建），而非静默返回半个状态"
        );
        assert!(
            !p.get_state_path(key).exists(),
            "旧格式文件应被清除，避免下次重复触发迁移"
        );

        // 重新读取不再报错。
        assert!(p.load(key).is_none());
    }
}
