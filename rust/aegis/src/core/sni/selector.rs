use once_cell::sync::Lazy;
use prost::Message;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
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

/// 从种子确定性重建排列，并跳过已消耗的 `used_count` 个。
fn build_order(len: usize, seed: u64, used_count: u32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    let mut rng = SmallRng::seed_from_u64(seed);
    order.shuffle(&mut rng);

    // used_count 可能超过当前域名数（例如 .pb 更新后变短），不能直接索引。
    let skip = (used_count as usize).min(len);
    order.truncate(len - skip);
    order
}

pub struct SNISelector {
    domains: Vec<String>,
    /// 运行期缓存的剩余排列。**只存在于内存**，不落盘。
    shuffled_indices: Vec<usize>,
    used_count: u32,
    /// 本轮种子，用于确定性重建 `shuffled_indices`。
    seed: u64,
    cache_key: String,
    /// 是否有尚未落盘的进度。
    dirty: bool,
}

impl SNISelector {
    pub async fn get_for_country(country_code: &str) -> Self {
        let upper = country_code.to_uppercase();
        let code = match upper.as_str() {
            "UK" => "GB",
            c => c,
        };

        let domains = Self::load_domains(code).await;

        let cache_key = format!("sni_{}", code);

        if let Some(ref persistence) = *SNI_PERSISTENCE
            && let Some(state) = persistence.load(&cache_key)
        {
            let total = domains.len();
            let shuffled_indices = build_order(total, state.seed, state.used_count);
            log::info!(
                "Loaded persisted SNI state for {}: {} domains, remaining={}, used={}",
                cache_key,
                total,
                shuffled_indices.len(),
                state.used_count
            );
            return Self {
                domains,
                shuffled_indices,
                used_count: state.used_count,
                seed: state.seed,
                cache_key,
                dirty: false,
            };
        }

        let seed = rand::rng().random::<u64>();
        let mut selector = Self {
            shuffled_indices: build_order(domains.len(), seed, 0),
            domains,
            used_count: 0,
            seed,
            cache_key,
            dirty: true,
        };
        selector.persist();
        selector
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
            self.start_new_round();
        }

        let idx = self
            .shuffled_indices
            .pop()
            .expect("shuffled_indices should not be empty after reset");
        self.used_count += 1;
        // 关键：这里**不再落盘**。
        // 修复前每次抽样都重写完整状态（真机 19,591,273 字节），
        // 而这些数据完全可以由 seed + used_count 重建。
        self.dirty = true;

        self.domains[idx].clone()
    }

    /// 域名耗尽：换新种子开启下一轮，并立即落盘。
    fn start_new_round(&mut self) {
        self.seed = rand::rng().random::<u64>();
        self.used_count = 0;
        self.shuffled_indices = build_order(self.domains.len(), self.seed, 0);
        log::info!(
            "SNI rotation exhausted, starting new round with seed {} ({} domains)",
            self.seed,
            self.domains.len()
        );
        self.dirty = true;
        self.persist();
    }

    /// 落盘当前轮转状态。失败只告警 —— 状态丢失的后果是「可能重复用域名」，
    /// 而非功能不可用，不应因此中断业务。
    fn persist(&mut self) {
        if self.cache_key.is_empty() || !self.dirty {
            return;
        }
        if let Some(ref persistence) = *SNI_PERSISTENCE {
            let state = SNIState::new(self.seed, self.used_count);
            if let Err(e) = persistence.save(&self.cache_key, &state) {
                log::warn!("Failed to save SNI state: {}", e);
                return;
            }
            self.dirty = false;
        }
    }

    pub fn remaining(&self) -> usize {
        self.shuffled_indices.len()
    }

    pub fn total_used(&self) -> u32 {
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

impl Drop for SNISelector {
    /// 进程退出或 selector 被丢弃时保存未落盘的进度。
    ///
    /// 这是「不在每次抽样时落盘」与「不丢进度」之间的折中：
    /// 崩溃最多丢失本轮最后一批（< 1KB）的进度。
    fn drop(&mut self) {
        self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::Ordering;

    /// 把 SNI 状态目录重定向到临时目录。
    ///
    /// 必须在首次触碰 `SNI_PERSISTENCE`（一个 `once_cell::Lazy`）之前调用：
    /// 它每进程只解析一次目录。本模块用例均带 `#[serial]`，不会互相踩踏。
    fn use_temp_config_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().expect("创建临时目录");
        // SAFETY: nextest 每测试独立进程；单进程下由 #[serial] 保证不并发。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", dir.path()) };
        dir
    }

    // 以下用例共享 `DECODE_PROBE`（进程级全局计数器），必须串行执行，
    // 否则 `cargo test` 的多线程会互相污染计数。nextest 每测试独立进程，天然隔离。
    #[tokio::test]
    #[serial]
    async fn get_for_country_returns_selector_with_domains() {
        let _tmp = use_temp_config_dir();
        let selector = SNISelector::get_for_country("US").await;
        let mut s = selector;
        let first = s.get_next();
        assert!(!first.is_empty());
        assert!(first.contains('.'));
    }

    #[tokio::test]
    #[serial]
    async fn get_for_country_unknown_falls_back_to_default() {
        let _tmp = use_temp_config_dir();
        let selector = SNISelector::get_for_country("XX").await;
        let mut s = selector;
        let d = s.get_next();
        assert!(!d.is_empty());
    }

    /// 构造一个受控的 selector：给定域名与确定的剩余排列。
    fn test_selector(domains: &[&str], shuffled_indices: Vec<usize>) -> SNISelector {
        SNISelector {
            domains: domains.iter().map(|d| d.to_string()).collect(),
            shuffled_indices,
            used_count: 0,
            seed: 0,
            // cache_key 留空 → persist() 直接返回，测试不会碰磁盘。
            cache_key: String::new(),
            dirty: false,
        }
    }

    #[test]
    fn next_random_no_repeat() {
        let mut selector = test_selector(
            &["a.com", "b.com", "c.com", "d.com", "e.com"],
            vec![0, 1, 2, 3, 4],
        );

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
        let mut selector = test_selector(&["a.com", "b.com"], vec![0, 1]);

        selector.get_next();
        selector.get_next();
        assert_eq!(selector.remaining(), 0);

        selector.get_next();
        assert_eq!(selector.remaining(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn get_for_country_uk_normalizes_to_gb() {
        let _tmp = use_temp_config_dir();
        let selector_uk = SNISelector::get_for_country("UK").await;
        let selector_gb = SNISelector::get_for_country("GB").await;
        let mut s1 = selector_uk;
        let mut s2 = selector_gb;
        assert!(!s1.get_next().is_empty());
        assert!(!s2.get_next().is_empty());
    }

    #[test]
    fn remaining_count() {
        let mut selector = test_selector(&["a.com", "b.com", "c.com"], vec![0, 1, 2]);

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

    // ---- T4 / T5: 确定性重建与不再逐次落盘 ----

    /// T5: 同一 seed 必须重建出**完全相同**的排列。
    ///
    /// 这是「只存 seed + used_count」的正确性基础：进程重启后靠它
    /// 接上上一轮的进度，而不会重发已用过的域名。
    #[test]
    fn t5_build_order_is_deterministic_for_same_seed() {
        for seed in [0u64, 1, u64::MAX, 0xDEAD_BEEF_CAFE_F00D] {
            let a = build_order(1000, seed, 0);
            let b = build_order(1000, seed, 0);
            assert_eq!(a, b, "seed={seed} 两次重建结果不同");

            let c = build_order(1000, seed.wrapping_add(1), 0);
            assert_ne!(a, c, "不同 seed 不应产生相同排列");
        }
    }

    /// T5b: used_count 必须精确地「跳过已消耗的前缀」，而不是重置轮转。
    #[test]
    fn t5_build_order_skips_consumed_prefix() {
        let full = build_order(100, 42, 0);
        let after_37 = build_order(100, 42, 37);

        assert_eq!(after_37.len(), 63);
        assert_eq!(
            after_37,
            full[..full.len() - 37].to_vec(),
            "恢复后的剩余集合应等于原排列去掉已消耗的前缀"
        );
    }

    /// T5c: used_count 超过域名数时不能 panic（.pb 更新后变短的情况）。
    #[test]
    fn t5_build_order_tolerates_used_count_overflow() {
        let o = build_order(10, 42, 9999);
        assert!(o.is_empty(), "越界的 used_count 应退化为空排列而非 panic");
    }

    /// T4: 连续抽样不得产生任何落盘。
    ///
    /// 修复前 `get_next()` 每次都重写完整状态（真机 19,591,273 字节）。
    /// 这里用 mtime 不变来断言「没写」—— 比断言耗时稳定。
    #[tokio::test]
    #[serial]
    async fn t4_drawing_does_not_write_state_file() {
        let _tmp = use_temp_config_dir();

        let selector = SNISelector::get_for_country("US").await;
        // 确保初始状态已落盘，才能观察「后续抽样不再改写」。
        drop(selector);

        let path = std::path::Path::new(&_tmp.path().to_string_lossy().to_string())
            .join("sni_state")
            .join("sni_US.enc");
        assert!(path.exists(), "构造 selector 时应已写入初始状态");

        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let mut s = SNISelector::get_for_country("US").await;
        for _ in 0..10 {
            let sni = s.get_next();
            assert!(!sni.is_empty());
        }

        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "10 次抽样期间状态文件被改写 —— 落盘仍未消除");
    }
}
