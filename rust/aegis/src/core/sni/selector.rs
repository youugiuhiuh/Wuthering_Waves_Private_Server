use once_cell::sync::Lazy;
use prost::Message;
use rand::rng;
use rand::seq::SliceRandom;
use rust_embed::RustEmbed;

use super::state::{SNIPersistence, SNIState};

pub mod sni_proto {
    include!(concat!(env!("OUT_DIR"), "/sni.rs"));
}

#[derive(RustEmbed)]
#[folder = "src/resources/sni/"]
struct SniAssets;

static SNI_PERSISTENCE: Lazy<Option<SNIPersistence>> = Lazy::new(|| match SNIPersistence::new() {
    Ok(p) => Some(p),
    Err(e) => {
        log::warn!("SNIPersistence init failed, using memory-only: {}", e);
        None
    }
});

/// 编译期常量：域数最多的 SNI 文件及其域名数，由 `build.rs` 扫描得出。
///
/// 原先这里是一个 `Lazy`，首次 fallback 时会把 172 个 .pb **全部解码**来找最大值
/// （实测峰值 305 MB）。这类与运行期无关的构建期事实不该由线上进程重算，
/// 故改为在编译期求值。
const LARGEST_PB: &str = env!("AEGIS_LARGEST_PB");
const LARGEST_PB_COUNT: &str = env!("AEGIS_LARGEST_PB_COUNT");

fn load_protobuf(data: &[u8]) -> Option<Vec<String>> {
    #[cfg(test)]
    DECODE_PROBE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    sni_proto::DomainList::decode(data)
        .ok()
        .map(|dl| dl.domains)
}

/// 测试专用：protobuf **解码**次数。
///
/// 选 `load_protobuf` 作为埋点而不是 `load_embedded_async`：前者是所有解码的
/// 唯一必经之路（旧的 `find_file_with_most_domains` 也走它），后者可被绕过。
#[cfg(test)]
static DECODE_PROBE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct SNISelector {
    domains: Vec<String>,
    shuffled_indices: Vec<usize>,
    used_count: usize,
    cache_key: String,
}

impl SNISelector {
    pub async fn get_for_country(country_code: &str) -> Self {
        let upper = country_code.to_uppercase();
        let code = match upper.as_str() {
            "UK" => "GB",
            c => c,
        };

        let domains = Self::load_domains(code).await;
        let state = SNIState::new(domains.clone());

        let cache_key = format!("sni_{}", code);

        if let Some(ref persistence) = *SNI_PERSISTENCE
            && let Some(state) = persistence.load(&cache_key)
        {
            log::info!(
                "Loaded persisted SNI state for {}: {} domains, remaining={}, used={}",
                cache_key,
                state.domains.len(),
                state.shuffled_indices.len(),
                state.used_count
            );
            return Self {
                domains: state.domains,
                shuffled_indices: state.shuffled_indices,
                used_count: state.used_count,
                cache_key,
            };
        }

        if let Some(ref persistence) = *SNI_PERSISTENCE
            && let Err(e) = persistence.save(&cache_key, &state)
        {
            log::warn!("Failed to save initial SNI state: {}", e);
        }

        Self {
            domains: state.domains,
            shuffled_indices: state.shuffled_indices,
            used_count: state.used_count,
            cache_key,
        }
    }

    async fn load_domains(code: &str) -> Vec<String> {
        const MIN_DOMAINS: usize = 3;

        let code_upper = code.to_uppercase();
        let pb_file = format!("{}.pb", code_upper);

        let country_domains = Self::load_embedded_async(&pb_file).await;

        if let Some(domains) = country_domains {
            if domains.len() >= MIN_DOMAINS {
                return domains;
            }
            log::warn!(
                "SNI file for {} has only {} domains (< {}), falling back to file with most domains",
                code_upper,
                domains.len(),
                MIN_DOMAINS
            );
        }

        log::info!(
            "Using fallback file: {} ({} domains)",
            LARGEST_PB,
            LARGEST_PB_COUNT
        );
        if let Some(domains) = Self::load_embedded_async(LARGEST_PB).await {
            return domains;
        }

        // R4: 走到这里意味着 REALITY 的伪装目标会是空的，属于功能降级。
        // 旧实现在此静默返回 vec![]，只留一行 log::info，极难发现。
        log::error!(
            "SNI fallback 失败：无法解码编译期选定的最大文件 {LARGEST_PB}，\
             REALITY 伪装目标将为空（功能已降级）"
        );

        vec![]
    }

    pub fn get_next(&mut self) -> String {
        if self.domains.is_empty() {
            return String::new();
        }

        if self.shuffled_indices.is_empty() {
            self.reset_shuffled_indices();
            self.save_state();
        }

        let idx = self
            .shuffled_indices
            .pop()
            .expect("shuffled_indices should not be empty after reset");
        self.used_count += 1;
        self.save_state();

        self.domains[idx].clone()
    }

    fn reset_shuffled_indices(&mut self) {
        let mut indices: Vec<usize> = (0..self.domains.len()).collect();
        let mut rng = rng();
        indices.shuffle(&mut rng);
        self.shuffled_indices = indices;
    }

    fn save_state(&self) {
        if self.cache_key.is_empty() {
            return;
        }
        if let Some(ref persistence) = *SNI_PERSISTENCE {
            let state = SNIState {
                domains: self.domains.clone(),
                shuffled_indices: self.shuffled_indices.clone(),
                used_count: self.used_count,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = persistence.save(&self.cache_key, &state) {
                log::warn!("Failed to save SNI state: {}", e);
            }
        }
    }

    pub fn remaining(&self) -> usize {
        self.shuffled_indices.len()
    }

    pub fn total_used(&self) -> usize {
        self.used_count
    }

    async fn load_embedded_async(filename: &str) -> Option<Vec<String>> {
        let file = SniAssets::get(filename)?;
        let data = file.data.into_owned();
        tokio::task::spawn_blocking(move || load_protobuf(&data))
            .await
            .ok()?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::Ordering;

    // 以下用例共享 `DECODE_PROBE`（进程级全局计数器），必须串行执行，
    // 否则 `cargo test` 的多线程会互相污染计数。nextest 每测试独立进程，天然隔离。
    #[tokio::test]
    #[serial]
    async fn get_for_country_returns_selector_with_domains() {
        let selector = SNISelector::get_for_country("US").await;
        let mut s = selector;
        let first = s.get_next();
        assert!(!first.is_empty());
        assert!(first.contains('.'));
    }

    #[tokio::test]
    #[serial]
    async fn get_for_country_unknown_falls_back_to_default() {
        let selector = SNISelector::get_for_country("XX").await;
        let mut s = selector;
        let d = s.get_next();
        assert!(!d.is_empty());
    }

    #[test]
    fn next_random_no_repeat() {
        let mut selector = SNISelector {
            domains: vec![
                "a.com".to_string(),
                "b.com".to_string(),
                "c.com".to_string(),
                "d.com".to_string(),
                "e.com".to_string(),
            ],
            shuffled_indices: vec![0, 1, 2, 3, 4],
            used_count: 0,
            cache_key: String::new(),
        };

        let mut results = Vec::new();
        for _ in 0..5 {
            results.push(selector.get_next());
        }

        let unique: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(unique.len(), 5, "Should have 5 unique domains");
        assert_eq!(selector.remaining(), 0);
    }

    #[test]
    fn next_resets_when_exhausted() {
        let mut selector = SNISelector {
            domains: vec!["a.com".to_string(), "b.com".to_string()],
            shuffled_indices: vec![0, 1],
            used_count: 0,
            cache_key: String::new(),
        };

        selector.get_next();
        selector.get_next();
        assert_eq!(selector.remaining(), 0);

        selector.get_next();
        assert_eq!(selector.remaining(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn get_for_country_uk_normalizes_to_gb() {
        let selector_uk = SNISelector::get_for_country("UK").await;
        let selector_gb = SNISelector::get_for_country("GB").await;
        let mut s1 = selector_uk;
        let mut s2 = selector_gb;
        assert!(!s1.get_next().is_empty());
        assert!(!s2.get_next().is_empty());
    }

    #[test]
    fn remaining_count() {
        let mut selector = SNISelector {
            domains: vec![
                "a.com".to_string(),
                "b.com".to_string(),
                "c.com".to_string(),
            ],
            shuffled_indices: vec![0, 1, 2],
            used_count: 0,
            cache_key: String::new(),
        };

        assert_eq!(selector.remaining(), 3);
        selector.get_next();
        assert_eq!(selector.remaining(), 2);
        selector.get_next();
        assert_eq!(selector.remaining(), 1);
    }

    #[test]
    #[serial]
    fn load_protobuf_decodes_valid_data() {
        let domains = vec!["example.com".to_string(), "test.com".to_string()];
        let list = sni_proto::DomainList { domains };
        let mut buf = Vec::new();
        list.encode(&mut buf).unwrap();
        let decoded = load_protobuf(&buf).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0], "example.com");
        assert_eq!(decoded[1], "test.com");
    }

    /// T1: build.rs 导出的常量必须与运行期对该文件的实际解码结果一致。
    ///
    /// 这条用例同时是 build.rs 里手写 prost 消息类型的**漂移防护**：
    /// 若 `proto/sni.proto` 增删字段导致编译期与运行期解析出不同的域数，
    /// 断言会在这里失败，而不是让 fallback 静默挑错文件。
    #[test]
    #[serial]
    fn t1_build_time_largest_matches_runtime_decode() {
        let largest = env!("AEGIS_LARGEST_PB");
        let count: usize = env!("AEGIS_LARGEST_PB_COUNT")
            .parse()
            .expect("常量应为数字");

        let file = SniAssets::get(largest).unwrap_or_else(|| panic!("{largest} 未被嵌入"));
        let decoded = load_protobuf(file.data.as_ref()).expect("最大文件应能解码");

        assert_eq!(
            decoded.len(),
            count,
            "编译期域数与运行期解码域数不一致，说明 build.rs 的手写消息类型与 proto 定义漂移"
        );
        assert!(count > 0, "最大文件不应为空");
    }

    /// T1b: 常量指向的文件必须真的比其余所有文件都大（防止取到 max 之外的值）。
    #[test]
    #[serial]
    fn t1_largest_is_actually_the_max() {
        let largest = env!("AEGIS_LARGEST_PB");
        let count: usize = env!("AEGIS_LARGEST_PB_COUNT")
            .parse()
            .expect("常量应为数字");

        for name in SniAssets::iter() {
            let name = name.as_ref();
            if !name.ends_with(".pb") || name == largest {
                continue;
            }
            let other = SniAssets::get(name).expect("嵌入文件应存在");
            if let Some(d) = load_protobuf(other.data.as_ref()) {
                assert!(
                    d.len() <= count,
                    "{name} 有 {} 个域名，超过编译期选出的 {largest}（{count}）",
                    d.len()
                );
            }
        }
    }

    /// T2: fallback 路径不得遍历并解码全部 .pb。
    ///
    /// 做法：`SniAssets::iter()` / `get()` 由 derive 生成、无法注入计数器，
    /// 因此在 `load_protobuf`（所有解码的唯一必经之路）上开一个计数器。
    /// 旧实现首次 fallback 解码 173 次（172 个候选 + 目标），新实现只解码 1 次。
    ///
    /// 注意：旧实现的全量扫描藏在 `once_cell::Lazy` 里，每进程只发生一次。
    /// 因此本用例必须以「进程内首次 fallback」的身份运行才有意义 ——
    /// `cargo nextest run` 每测试独立进程，天然满足；`cargo test` 下若其他
    /// 用例先跑过 fallback，Lazy 已预热，本用例会假通过。修复后此顾虑消失。
    #[tokio::test]
    #[serial]
    async fn t2_fallback_decodes_only_the_largest_file() {
        DECODE_PROBE.store(0, Ordering::Relaxed);
        let domains = SNISelector::load_domains("ZZ_NO_SUCH_COUNTRY").await;
        let decoded = DECODE_PROBE.load(Ordering::Relaxed);

        assert!(
            !domains.is_empty(),
            "fallback 必须返回非空域名，否则 REALITY 伪装目标会静默消失"
        );
        assert_eq!(
            decoded, 1,
            "fallback 只应解码最大文件一次，实际解码了 {decoded} 个"
        );
    }

    #[tokio::test]
    #[serial]
    async fn t2_load_domains_never_returns_empty_for_any_code() {
        // R4: 任何国家码（包括不存在的）都必须能拿到域名，取不到就是 error 级故障。
        for code in ["ZZ", "", "QQ", "!!"] {
            let d = SNISelector::load_domains(code).await;
            assert!(
                !d.is_empty(),
                "国家码 {code:?} 的 fallback 返回了空列表 —— 伪装目标会消失"
            );
        }
    }
}
