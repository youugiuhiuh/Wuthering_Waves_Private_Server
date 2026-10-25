//! 存量 `.pb` 双池升级（对齐 Go `pkg/upgrade.go` 的 `UpgradePbFile` / `UpgradePbDir`）。
//!
//! 用途：把 v1.9.9 时代的单池 `DomainList{domains}` 原地转成 `{pq_domains, classic_domains}`，
//! 免去从头重跑整个采集流程。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use crate::cloudflare;
use crate::dns;
use crate::geo;
use crate::pb;
use crate::progress;
use crate::storage::Storage;
use crate::tls;

/// 单文件升级统计（对齐 Go `UpgradeStats`）。
#[derive(Debug, Default, Clone)]
pub struct UpgradeStats {
    #[allow(dead_code)] // 逐个文件的来源路径，供调用方 / 报表使用
    pub file_path: PathBuf,
    pub total: usize,
    pub pq_count: usize,
    pub classic_count: usize,
    pub dead_count: usize,
    pub elapsed: Duration,
}

/// 升级参数。
pub struct UpgradeOptions {
    pub workers: usize,
    pub timeout: Duration,
    /// 是否把失败域名保留到 classic 池（Go `-keep-dead`）
    pub keep_dead: bool,
    /// 是否跳过近期失败过的域名（需要 `storage`）
    pub storage: Option<Arc<Storage>>,
    pub force: bool,
    /// 源文件格式；旧存量 `.pb` 必须显式声明为 `LegacySinglePool`
    pub format: PbFormat,
}

impl Default for UpgradeOptions {
    fn default() -> Self {
        Self {
            workers: 30,
            timeout: Duration::from_secs(5),
            keep_dead: false,
            storage: None,
            force: false,
            format: PbFormat::DualPool,
        }
    }
}

/// 升级单个 `.pb`。`out_path` 为空时原地覆盖。
pub async fn upgrade_pb_file(
    pb_path: &Path,
    out_path: Option<&Path>,
    geodb: Option<Arc<geo::GeoDb>>,
    opts: UpgradeOptions,
) -> Result<UpgradeStats, Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let data = std::fs::read(pb_path)?;

    // 兼容两种格式：已是双池则只取已有两池；旧单池则退化为 classic
    let (existing_pq, existing_classic) = decode_as(&data, opts.format)
        .map_err(|e| format!("解析 {} 失败: {e}", pb_path.display()))?;

    let mut domains: Vec<String> = existing_pq
        .iter()
        .chain(existing_classic.iter())
        .cloned()
        .collect();
    domains.sort();
    domains.dedup();

    let total = domains.len();
    let target = out_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| pb_path.to_path_buf());

    if total == 0 {
        return Ok(UpgradeStats {
            file_path: pb_path.to_path_buf(),
            elapsed: start.elapsed(),
            ..Default::default()
        });
    }

    let workers = opts.workers.max(1);
    let resolver = Arc::new(dns::build_resolver(Duration::from_millis(2500))?);
    let sem = Arc::new(Semaphore::new(workers));
    let pb_bar = progress::bar(total as u64, "upgrade");

    let mut pq_out: Vec<String> = Vec::new();
    let mut classic_out: Vec<String> = Vec::new();
    let dead = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::with_capacity(domains.len());
    for domain in domains {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let resolver = resolver.clone();
        let geodb = geodb.clone();
        let storage = opts.storage.clone();
        let timeout = opts.timeout;
        let force = opts.force;
        let bar = pb_bar.clone();
        let dead = dead.clone();

        handles.push(tokio::task::spawn_blocking(move || {
            // 近期失败跳过（除非 force）
            if !force
                && storage
                    .as_ref()
                    .is_some_and(|s| s.is_failed_recently(&domain))
            {
                dead.fetch_add(1, Ordering::Relaxed);
                bar.inc(1);
                drop(permit);
                return (None, false);
            }

            // 解析 IP（阶段一已缓存，此处仅补齐）
            let ip = {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async { dns::resolve_first(&resolver, &domain).await.ok() })
            };
            let Some(ip) = ip else {
                dead.fetch_add(1, Ordering::Relaxed);
                bar.inc(1);
                drop(permit);
                return (None, false);
            };

            if cloudflare::is_cloudflare_ip(ip) {
                dead.fetch_add(1, Ordering::Relaxed);
                bar.inc(1);
                drop(permit);
                return (None, false);
            }

            // 管辖国在握手前先判一次，省掉无谓握手
            if let Some(g) = geodb.as_ref()
                && geo::is_blocked_country(&g.country(ip))
            {
                dead.fetch_add(1, Ordering::Relaxed);
                bar.inc(1);
                drop(permit);
                return (None, false);
            }

            let result = match tls::probe(&domain, ip, timeout) {
                Ok(p) if tls::validate(&p) => {
                    if let Some(s) = storage.as_ref() {
                        let country = geodb
                            .as_ref()
                            .map(|g| g.country(ip))
                            .unwrap_or_else(|| "UNKNOWN".to_string());
                        let _ = s.save_success(&domain, &country);
                    }
                    Some(p.is_pq())
                }
                _ => {
                    if let Some(s) = storage.as_ref() {
                        let _ = s.append_failures(std::slice::from_ref(&domain));
                    }
                    dead.fetch_add(1, Ordering::Relaxed);
                    None
                }
            };
            let is_pq = result;
            bar.inc(1);
            drop(permit);
            (is_pq.map(|q| (domain, q)), is_pq.is_none())
        }));
    }

    for h in handles {
        if let Ok((Some((domain, is_pq)), _)) = h.await {
            if is_pq {
                pq_out.push(domain);
            } else {
                classic_out.push(domain);
            }
        }
    }

    // 保留失败项到 classic 池
    if opts.keep_dead {
        let all: Vec<String> = existing_classic.clone();
        for d in all {
            if !pq_out.contains(&d) && !classic_out.contains(&d) {
                classic_out.push(d);
            }
        }
    }

    pq_out.sort();
    pq_out.dedup();
    classic_out.sort();
    classic_out.dedup();

    std::fs::write(&target, pb::encode(&pq_out, &classic_out))?;

    let stats = UpgradeStats {
        file_path: pb_path.to_path_buf(),
        total,
        pq_count: pq_out.len(),
        classic_count: classic_out.len(),
        dead_count: dead.load(Ordering::Relaxed) as usize,
        elapsed: start.elapsed(),
    };

    progress::finish(
        &pb_bar,
        &format!(
            "  {} -> {} | total={} PQ={} classic={} dead={} ({:.1}s)",
            pb_path.display(),
            target.display(),
            stats.total,
            stats.pq_count,
            stats.classic_count,
            stats.dead_count,
            stats.elapsed.as_secs_f64()
        ),
    );

    Ok(stats)
}

/// 升级目录下所有 `.pb`。按文件大小升序处理，让小国先秒完。
pub async fn upgrade_pb_dir(
    dir: &Path,
    out_dir: Option<&Path>,
    geodb: Option<Arc<geo::GeoDb>>,
    opts: UpgradeOptions,
) -> Result<Vec<UpgradeStats>, Box<dyn std::error::Error + Send + Sync>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pb"))
        .collect();

    if files.is_empty() {
        return Err(format!("{} 下没有 .pb 文件", dir.display()).into());
    }

    if let Some(od) = out_dir {
        std::fs::create_dir_all(od)?;
    }

    files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));

    let total_files = files.len();
    let mut all = Vec::with_capacity(total_files);

    for (idx, f) in files.iter().enumerate() {
        let target = out_dir.map(|od| od.join(f.file_name().unwrap_or_default()));
        println!(
            "[{}/{}] 升级 {} ...",
            idx + 1,
            total_files,
            f.file_name().unwrap_or_default().to_string_lossy()
        );
        match upgrade_pb_file(
            f,
            target.as_deref(),
            geodb.clone(),
            UpgradeOptions {
                workers: opts.workers,
                timeout: opts.timeout,
                keep_dead: opts.keep_dead,
                storage: opts.storage.clone(),
                force: opts.force,
                format: opts.format,
            },
        )
        .await
        {
            Ok(s) => all.push(s),
            Err(e) => eprintln!("  升级 {} 失败: {e}", f.display()),
        }
    }

    Ok(all)
}

/// 源 `.pb` 的格式。
///
/// 旧版 (`domains = 1`) 与新版的 `pq_domains = 1` **字段号完全相同**，
/// 字节层面无法区分。因此旧格式必须显式声明，绝不能靠启发式猜
/// —— 猜错的后果是把整池 classic 域名误标成 PQ，客户端会按 PQ 握手失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbFormat {
    /// 双池（pq_domains=1, classic_domains=2）
    DualPool,
    /// v1.9.9 单池（domains=1），全部按 classic 处理
    LegacySinglePool,
}

/// 按指定格式解码 `.pb`。
pub fn decode_as(data: &[u8], format: PbFormat) -> Result<(Vec<String>, Vec<String>), String> {
    let (field1, field2) = pb::decode(data)?;
    match format {
        PbFormat::DualPool => Ok((field1, field2)),
        // 旧格式全部进 classic，pq 留空
        PbFormat::LegacySinglePool => {
            let mut classic = field1;
            classic.extend(field2);
            Ok((Vec::new(), classic))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("upg_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn decodes_dual_pool_file() {
        let data = pb::encode(&["pq.com".to_string()], &["cls.com".to_string()]);
        let (pq, classic) = decode_as(&data, PbFormat::DualPool).unwrap();
        assert_eq!(pq, vec!["pq.com"]);
        assert_eq!(classic, vec!["cls.com"]);
    }

    /// 旧单池必须显式声明格式；声明为 LegacySinglePool 后全部进 classic。
    #[test]
    fn legacy_single_pool_becomes_classic() {
        let data = pb::encode(&["old1.com".to_string(), "old2.com".to_string()], &[]);
        let (pq, classic) = decode_as(&data, PbFormat::LegacySinglePool).unwrap();
        assert!(pq.is_empty(), "旧单池不应被误判为 PQ");
        assert_eq!(classic, vec!["old1.com", "old2.com"]);
    }

    /// 回归：把旧文件按 DualPool 误读会得到"全 PQ"的错误结果，
    /// 这正是不能靠启发式猜格式的原因。
    #[test]
    fn misreading_legacy_as_dual_pool_is_detectably_wrong() {
        let data = pb::encode(&["old.com".to_string()], &[]);
        let (pq_wrong, _) = decode_as(&data, PbFormat::DualPool).unwrap();
        let (pq_right, classic_right) = decode_as(&data, PbFormat::LegacySinglePool).unwrap();
        assert_eq!(pq_wrong, vec!["old.com"], "误读时字段会落进 pq 池");
        assert!(pq_right.is_empty());
        assert_eq!(classic_right, vec!["old.com"]);
    }

    #[test]
    fn empty_file_yields_empty_stats() {
        let dir = tmp("empty");
        let f = dir.join("EMPTY.pb");
        std::fs::write(&f, b"").unwrap();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let stats = rt
            .block_on(upgrade_pb_file(&f, None, None, UpgradeOptions::default()))
            .unwrap();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.pq_count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upgrade_dir_rejects_empty_directory() {
        let dir = tmp("nodir");
        let err = upgrade_pb_dir(&dir, None, None, UpgradeOptions::default()).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("没有 .pb"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upgrade_dir_processes_all_files_to_out_dir() {
        let dir = tmp("outdir");
        let src = dir.join("src");
        let out = dir.join("out");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("AA.pb"), pb::encode(&["a.com".to_string()], &[])).unwrap();
        std::fs::write(src.join("BB.pb"), pb::encode(&["b.com".to_string()], &[])).unwrap();
        std::fs::write(src.join("skip.txt"), b"x").unwrap();

        let stats = upgrade_pb_dir(&src, Some(&out), None, UpgradeOptions::default())
            .await
            .unwrap();
        assert_eq!(stats.len(), 2, "应只处理 .pb 文件");
        assert!(out.join("AA.pb").exists());
        assert!(out.join("BB.pb").exists());
        assert!(!out.join("skip.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
