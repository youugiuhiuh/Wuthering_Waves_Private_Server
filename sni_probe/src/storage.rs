//! 历史存储（对应 Go 版 `pkg/storage.go` 的 badger 实现）。
//!
//! 用 `redb`（纯 Rust 嵌入式 KV）替代 badger，语义对齐：
//!
//! | 前缀 | 键 | 值 | TTL |
//! | :--- | :--- | :--- | :--- |
//! | `failed:` | 域名 | 上次失败 Unix 秒（u64 LE） | ttl_days |
//! | `success:` | 域名 | JSON `SuccessInfo` | ttl_days |
//! | `blocked:country:` | 域名 | 阻塞原因 | 永久 |
//! | `blocked:asn:` | 域名 | 阻塞原因 | 永久 |
//! | `asn:` | ASN 十进制 | JSON `AsnInfo` | ttl_days |
//!
//! redb 没有原生 TTL，因此写入时记录到期时间戳，读取时自行过滤。

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use redb::{Database, ReadableTable, TableDefinition};

const FAILED: TableDefinition<&str, u64> = TableDefinition::new("failed");
const FAILED_EXPIRY: TableDefinition<&str, u64> = TableDefinition::new("failed_expiry");
const SUCCESS: TableDefinition<&str, &str> = TableDefinition::new("success");
const SUCCESS_EXPIRY: TableDefinition<&str, u64> = TableDefinition::new("success_expiry");
const BLOCKED_COUNTRY: TableDefinition<&str, &str> = TableDefinition::new("blocked_country");
const BLOCKED_ASN: TableDefinition<&str, &str> = TableDefinition::new("blocked_asn");
const ASN: TableDefinition<&str, &str> = TableDefinition::new("asn");
const ASN_EXPIRY: TableDefinition<&str, u64> = TableDefinition::new("asn_expiry");

/// 已成功目标的记录。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SuccessInfo {
    pub domain: String,
    pub country: String,
    pub tested_at: i64,
}

/// 被封禁 ASN 的记录。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AsnInfo {
    pub org: String,
    pub country: String,
    pub added_at: i64,
}

/// 种子封禁 ASN：境内云厂商 / 运营商，与 Go 版 `SeedBlockedASNs` 一致。
///
/// 这些 ASN 下的 IP 都属于审查管辖地，用作 REALITY 伪装目标会造成流量反向异常。
pub const SEED_BLOCKED_ASNS: &[(u32, &str)] = &[
    (45102, "Alibaba US Technology Co., Ltd."),
    (
        55967,
        "Beijing Baidu Netcom Science and Technology Co., Ltd.",
    ),
    (132203, "Tencent Building, Kejizhongyi Avenue"),
    (4808, "China Unicom Beijing Province Network"),
    (4134, "Chinanet"),
    (4811, "China Telecom Group"),
    (38283, "CHINANET SiChuan Telecom Internet Data Center"),
    (24151, "China Internet Network Infomation Center"),
    (21859, "Zenlayer Inc"),
    (17623, "China Telecom (Beijing) IDC"),
    (4837, "China Unicom IP network"),
    (58466, "Shanghai Bell Networks"),
    (45275, "Tencent Cloud Computing (Beijing)"),
    (58772, "Alibaba (Beijing) Technology"),
    (24542, "Chinanet IDC"),
    (23650, "Chinanet Jiangsu Province Network"),
    (23764, "Chinanet Guangdong Province Network"),
    (4538, "China Telecom Group"),
    (38176, "Tencent Cloud"),
    (17429, "Tencent Cloud Computing"),
    (149979, "Tencent Cloud Computing"),
    (45081, "Tencent Cloud Computing"),
    (141024, "Tencent Cloud Computing"),
    (136190, "Tencent Cloud"),
    (212238, "Tencent Cloud"),
];

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 历史存储句柄。
pub struct Storage {
    db: Database,
    ttl_secs: u64,
}

impl Storage {
    /// 打开（或新建）历史库。`ttl_days` 对齐 Go 版 `-ttl`。
    pub fn open(
        path: impl AsRef<Path>,
        ttl_days: u32,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let db = Database::create(path)?;

        // 首次打开即建表，避免后续 write 事务里现建
        let tx = db.begin_write()?;
        {
            tx.open_table(FAILED)?;
            tx.open_table(FAILED_EXPIRY)?;
            tx.open_table(SUCCESS)?;
            tx.open_table(SUCCESS_EXPIRY)?;
            tx.open_table(BLOCKED_COUNTRY)?;
            tx.open_table(BLOCKED_ASN)?;
            tx.open_table(ASN)?;
            tx.open_table(ASN_EXPIRY)?;
        }
        tx.commit()?;

        Ok(Self {
            db,
            ttl_secs: u64::from(ttl_days) * 24 * 3600,
        })
    }

    /// 该域名是否在 TTL 内失败过（等价 Go `IsFailedRecently`）。
    pub fn is_failed_recently(&self, domain: &str) -> bool {
        let tx = match self.db.begin_read() {
            Ok(t) => t,
            Err(_) => return false,
        };
        let failed = match tx.open_table(FAILED) {
            Ok(t) => t,
            Err(_) => return false,
        };
        let expiry = match tx.open_table(FAILED_EXPIRY) {
            Ok(t) => t,
            Err(_) => return false,
        };

        let expired = match expiry.get(domain) {
            Ok(Some(v)) => v.value() < now_secs(),
            _ => return false,
        };
        if expired {
            return false;
        }
        matches!(failed.get(domain), Ok(Some(_)))
    }

    /// 批量记录失败（等价 Go `AppendFailureHistory`）。
    pub fn append_failures(
        &self,
        domains: &[String],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if domains.is_empty() {
            return Ok(());
        }
        let now = now_secs();
        let expire_at = now + self.ttl_secs;

        let tx = self.db.begin_write()?;
        {
            let mut failed = tx.open_table(FAILED)?;
            let mut expiry = tx.open_table(FAILED_EXPIRY)?;
            for d in domains {
                failed.insert(d.as_str(), now)?;
                expiry.insert(d.as_str(), expire_at)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 记录成功（等价 Go `SaveSuccess`）。
    pub fn save_success(
        &self,
        domain: &str,
        country: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = now_secs();
        let info = SuccessInfo {
            domain: domain.to_string(),
            country: country.to_string(),
            tested_at: now as i64,
        };
        let json = serde_json::to_string(&info)?;

        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(SUCCESS)?;
            t.insert(domain, json.as_str())?;
            let mut e = tx.open_table(SUCCESS_EXPIRY)?;
            e.insert(domain, now + self.ttl_secs)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 读取全部仍在 TTL 内的成功记录：域名 → 国家（等价 Go `LoadSuccessHistory`）。
    #[allow(dead_code)] // 公共 API：供统计/报表使用
    pub fn load_success_history(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Ok(tx) = self.db.begin_read() else {
            return out;
        };
        let (Ok(t), Ok(e)) = (tx.open_table(SUCCESS), tx.open_table(SUCCESS_EXPIRY)) else {
            return out;
        };

        let now = now_secs();
        let Ok(iter) = t.range::<&str>(..) else {
            return out;
        };
        for row in iter.flatten() {
            let (k, v) = row;
            let domain = k.value().to_string();
            let alive = matches!(e.get(domain.as_str()), Ok(Some(x)) if x.value() >= now);
            if !alive {
                continue;
            }
            if let Ok(info) = serde_json::from_str::<SuccessInfo>(v.value()) {
                out.insert(domain, info.country);
            }
        }
        out
    }

    /// 记录被封禁域名（国家或 ASN 触发）。
    pub fn add_blocked_domain(
        &self,
        domain: &str,
        reason: &str,
        by_asn: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tx = self.db.begin_write()?;
        {
            if by_asn {
                let mut t = tx.open_table(BLOCKED_ASN)?;
                t.insert(domain, reason)?;
            } else {
                let mut t = tx.open_table(BLOCKED_COUNTRY)?;
                t.insert(domain, reason)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 读取全部被封禁域名（等价 Go `LoadBlockedHistory`）。
    #[allow(dead_code)] // 公共 API：供统计使用
    pub fn load_blocked_history(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(tx) = self.db.begin_read() else {
            return out;
        };
        for table in [BLOCKED_COUNTRY, BLOCKED_ASN] {
            let Ok(t) = tx.open_table(table) else {
                continue;
            };
            let Ok(iter) = t.range::<&str>(..) else {
                continue;
            };
            for row in iter.flatten() {
                out.push(row.0.value().to_string());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// 把 ASN 加入封禁名单（等价 Go `AddASNToBlocklist`）。
    pub fn add_asn_to_blocklist(
        &self,
        asn: u32,
        org: &str,
        country: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = now_secs();
        let info = AsnInfo {
            org: org.to_string(),
            country: country.to_string(),
            added_at: now as i64,
        };
        let json = serde_json::to_string(&info)?;
        let key = asn.to_string();

        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(ASN)?;
            t.insert(key.as_str(), json.as_str())?;
            let mut e = tx.open_table(ASN_EXPIRY)?;
            e.insert(key.as_str(), now + self.ttl_secs)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 读取封禁 ASN 名单：种子 ∪ 库内未过期记录（等价 Go `LoadASNBlocklist`）。
    pub fn load_asn_blocklist(&self) -> HashMap<u32, AsnInfo> {
        let now = now_secs();
        let mut out: HashMap<u32, AsnInfo> = SEED_BLOCKED_ASNS
            .iter()
            .map(|(asn, org)| {
                (
                    *asn,
                    AsnInfo {
                        org: (*org).to_string(),
                        country: "SEED".to_string(),
                        added_at: now as i64,
                    },
                )
            })
            .collect();

        let Ok(tx) = self.db.begin_read() else {
            return out;
        };
        let (Ok(t), Ok(e)) = (tx.open_table(ASN), tx.open_table(ASN_EXPIRY)) else {
            return out;
        };
        let Ok(iter) = t.range::<&str>(..) else {
            return out;
        };
        for row in iter.flatten() {
            let (k, v) = row;
            let key = k.value();
            let alive = matches!(e.get(key), Ok(Some(x)) if x.value() >= now);
            if !alive {
                continue;
            }
            let Ok(asn) = key.parse::<u32>() else {
                continue;
            };
            if let Ok(info) = serde_json::from_str::<AsnInfo>(v.value()) {
                out.insert(asn, info);
            }
        }
        out
    }

    /// 清空全部历史（等价 Go `ClearAll`）。
    pub fn clear_all(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tx = self.db.begin_write()?;
        {
            tx.delete_table(FAILED)?;
            tx.delete_table(FAILED_EXPIRY)?;
            tx.delete_table(SUCCESS)?;
            tx.delete_table(SUCCESS_EXPIRY)?;
            tx.delete_table(BLOCKED_COUNTRY)?;
            tx.delete_table(BLOCKED_ASN)?;
            tx.delete_table(ASN)?;
            tx.delete_table(ASN_EXPIRY)?;

            tx.open_table(FAILED)?;
            tx.open_table(FAILED_EXPIRY)?;
            tx.open_table(SUCCESS)?;
            tx.open_table(SUCCESS_EXPIRY)?;
            tx.open_table(BLOCKED_COUNTRY)?;
            tx.open_table(BLOCKED_ASN)?;
            tx.open_table(ASN)?;
            tx.open_table(ASN_EXPIRY)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 当前有效（未过期）失败记录条数（等价 Go `CleanAndCountFailure` 的 active）。
    #[allow(dead_code)] // 公共 API：供统计使用
    pub fn count_active_failures(&self) -> usize {
        let now = now_secs();
        let Ok(tx) = self.db.begin_read() else {
            return 0;
        };
        let Ok(t) = tx.open_table(FAILED_EXPIRY) else {
            return 0;
        };
        let Ok(iter) = t.iter() else { return 0 };
        iter.flatten().filter(|r| r.1.value() >= now).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(tag: &str) -> (std::path::PathBuf, Storage) {
        let dir = std::env::temp_dir().join(format!("storage_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("hist.redb");
        let s = Storage::open(&path, 7).expect("open storage");
        (dir, s)
    }

    #[test]
    fn failure_within_ttl_is_recent() {
        let (dir, s) = tmp_db("fail");
        assert!(!s.is_failed_recently("a.com"));
        s.append_failures(&["a.com".to_string()]).unwrap();
        assert!(s.is_failed_recently("a.com"));
        assert!(!s.is_failed_recently("b.com"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_failure_is_not_recent() {
        let dir = std::env::temp_dir().join(format!("storage_exp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("hist.redb");
        // ttl_days = 0 => 立即过期
        let s = Storage::open(&path, 0).expect("open");
        s.append_failures(&["a.com".to_string()]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(!s.is_failed_recently("a.com"), "TTL 已过应不再算近期失败");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn success_history_roundtrips_country() {
        let (dir, s) = tmp_db("succ");
        s.save_success("a.com", "US").unwrap();
        s.save_success("b.com", "DE").unwrap();
        let h = s.load_success_history();
        assert_eq!(h.get("a.com").map(String::as_str), Some("US"));
        assert_eq!(h.get("b.com").map(String::as_str), Some("DE"));
        assert_eq!(h.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn asn_blocklist_includes_seeds_and_added() {
        let (dir, s) = tmp_db("asn");
        let list = s.load_asn_blocklist();
        // 种子必须在内，且条数与常量一致
        assert_eq!(list.len(), SEED_BLOCKED_ASNS.len());
        assert!(list.contains_key(&4134), "Chinanet 必须在种子名单");

        s.add_asn_to_blocklist(15169, "Google LLC", "US").unwrap();
        let list2 = s.load_asn_blocklist();
        assert_eq!(list2.len(), SEED_BLOCKED_ASNS.len() + 1);
        assert_eq!(list2[&15169].org, "Google LLC");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blocked_history_covers_country_and_asn() {
        let (dir, s) = tmp_db("blocked");
        s.add_blocked_domain("cn.com", "country CN", false).unwrap();
        s.add_blocked_domain("asn.com", "asn 4134", true).unwrap();
        let mut got = s.load_blocked_history();
        got.sort();
        assert_eq!(got, vec!["asn.com".to_string(), "cn.com".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_all_wipes_everything() {
        let (dir, s) = tmp_db("clear");
        s.append_failures(&["a.com".to_string()]).unwrap();
        s.save_success("b.com", "US").unwrap();
        s.add_blocked_domain("c.com", "x", false).unwrap();
        s.add_asn_to_blocklist(64500, "Test", "XX").unwrap();

        s.clear_all().unwrap();

        assert!(!s.is_failed_recently("a.com"));
        assert!(s.load_success_history().is_empty());
        assert!(s.load_blocked_history().is_empty());
        // 种子 ASN 应保留（它们来自常量而非库）
        assert_eq!(s.load_asn_blocklist().len(), SEED_BLOCKED_ASNS.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_active_failures_filters_expired() {
        let (dir, s) = tmp_db("count");
        s.append_failures(&["a.com".to_string(), "b.com".to_string()])
            .unwrap();
        assert_eq!(s.count_active_failures(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopens_existing_database() {
        let (dir, s) = tmp_db("reopen");
        s.save_success("persist.com", "JP").unwrap();
        let path = dir.join("hist.redb");
        drop(s);

        let s2 = Storage::open(&path, 7).expect("reopen");
        assert_eq!(
            s2.load_success_history()
                .get("persist.com")
                .map(String::as_str),
            Some("JP")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// ASN 黑名单的运行时视图：种子 ∪ 库内记录，支持同步累积。
///
/// 对齐 Go：同一 ASN 下累计失败域名数达到阈值，就把整个 AS 拉黑，
/// 后续同 ASN 目标直接跳过，避免在同一家 CDN 的几百个 IP 上重复浪费握手。
pub struct AsnBlocklist {
    blocked: std::sync::RwLock<(std::collections::HashSet<u32>, HashMap<u32, u32>)>,
    threshold: u32,
}

impl AsnBlocklist {
    /// `threshold`：某 ASN 累计失败多少个域名后整体拉黑（Go 默认 3）。
    pub fn new(storage: &Storage, threshold: u32) -> Self {
        let blocked: std::collections::HashSet<u32> =
            storage.load_asn_blocklist().into_keys().collect();
        Self {
            blocked: std::sync::RwLock::new((blocked, HashMap::new())),
            threshold,
        }
    }

    /// ASN 是否已被拉黑。
    pub fn is_blocked(&self, asn: u32) -> bool {
        self.blocked
            .read()
            .expect("blocklist poisoned")
            .0
            .contains(&asn)
    }

    /// 记录一次失败；达到阈值则返回 `true` 并完成拉黑。
    pub fn record_failure(&self, asn: u32) -> bool {
        let mut guard = self.blocked.write().expect("blocklist poisoned");
        let (blocked, counts) = &mut *guard;
        if blocked.contains(&asn) {
            return false;
        }
        let c = counts.entry(asn).or_insert(0);
        *c += 1;
        if *c >= self.threshold {
            blocked.insert(asn);
            return true;
        }
        false
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.blocked.read().expect("blocklist poisoned").0.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod asn_tests {
    use super::*;

    fn fresh(tag: &str) -> (std::path::PathBuf, Storage) {
        let dir = std::env::temp_dir().join(format!("asnbl_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("h.redb");
        let s = Storage::open(&path, 7).unwrap();
        (dir, s)
    }

    #[test]
    fn seeds_are_blocked_from_start() {
        let (dir, s) = fresh("seed");
        let bl = AsnBlocklist::new(&s, 3);
        assert!(bl.is_blocked(4134), "Chinanet 种子必须在名单");
        assert!(bl.is_blocked(4811), "China Telecom 种子必须在名单");
        assert!(!bl.is_blocked(15169), "Google 不应在种子名单");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blocks_only_after_threshold() {
        let (dir, s) = fresh("thresh");
        let bl = AsnBlocklist::new(&s, 3);

        assert!(!bl.record_failure(15169));
        assert!(!bl.is_blocked(15169));
        assert!(!bl.record_failure(15169));
        assert!(!bl.is_blocked(15169));

        assert!(bl.record_failure(15169), "第 3 次失败应触发拉黑");
        assert!(bl.is_blocked(15169));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failures_are_counted_per_asn() {
        let (dir, s) = fresh("per");
        let bl = AsnBlocklist::new(&s, 3);
        bl.record_failure(15169);
        bl.record_failure(15169);
        bl.record_failure(25000);
        bl.record_failure(25000);
        // 两个 ASN 各 2 次，都不该被拉黑
        assert!(!bl.is_blocked(15169));
        assert!(!bl.is_blocked(25000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn already_blocked_asn_is_not_double_counted() {
        let (dir, s) = fresh("dup");
        let bl = AsnBlocklist::new(&s, 1);
        assert!(bl.record_failure(15169));
        assert!(!bl.record_failure(15169), "已拉黑后不应再触发");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
