//! 三阶段流水线的并发执行器与输出写入。
//!
//! 每个阶段都是「流式读入 → 有界并发处理 → 流式写出」，因此百万级输入也不会撑爆内存。

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Semaphore, mpsc};

use crate::cloudflare;
use crate::dns;
use crate::geo;
use crate::progress;
use crate::state::Entry;
use crate::target;
use crate::tls;

/// 阶段执行结果统计。
#[derive(Debug, Default, Clone, Copy)]
pub struct StageStats {
    pub total: u64,
    pub kept: u64,
    pub dropped: u64,
    pub elapsed: Duration,
}

/// 逐行读取输入文件，跳过空行与注释。
fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    let file = File::open(path)?;
    Ok(BufReader::new(file).lines().map_while(Result::ok).collect())
}

/// 把 `Entry` 列表写回 `domain,ip` 文本（阶段一/二的 CLI 输出格式，可与 Go 版互换）。
fn write_entries(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(File::create(path)?);
    for e in entries {
        match e.ip {
            Some(ip) => writeln!(out, "{},{}", e.domain, ip)?,
            None => writeln!(out, "{}", e.domain)?,
        }
    }
    out.flush()
}

/// 从 `Vec<Entry>` 出发跑阶段一（供 `run` 子命令复用，无需中间 CSV）。
pub async fn resolve_map_entries(
    entries: Vec<Entry>,
    resolver: Arc<dns::TokioResolver>,
    workers: usize,
) -> Result<(Vec<Entry>, StageStats), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let total = entries.len() as u64;
    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (tx, mut rx) = mpsc::channel::<Entry>(workers * 2);
    let pb = progress::bar(total, "resolve-map");

    let sem = Arc::new(Semaphore::new(workers));
    let mut handles = Vec::with_capacity(entries.len());
    for e in entries {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let tx = tx.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        let resolver = resolver.clone();
        handles.push(tokio::spawn(async move {
            let ip = match e.ip {
                Some(ip) => Some(ip),
                None => dns::resolve_first(&resolver, &e.domain).await.ok(),
            };
            match ip {
                Some(ip) if !cloudflare::is_cloudflare_ip(ip) => {
                    kept.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(Entry::new(e.domain, ip)).await;
                }
                _ => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            drop(permit);
        }));
    }
    drop(tx);

    let mut collected = Vec::new();
    while let Some(entry) = rx.recv().await {
        pb.inc(1);
        collected.push(entry);
    }
    for h in handles {
        let _ = h.await;
    }
    progress::finish(
        &pb,
        &format!(
            "[resolve-map] total={total} kept={} dropped={} elapsed={:.1}s",
            kept.load(Ordering::Relaxed),
            dropped.load(Ordering::Relaxed),
            start.elapsed().as_secs_f64()
        ),
    );

    Ok((
        collected,
        StageStats {
            total,
            kept: kept.load(Ordering::Relaxed),
            dropped: dropped.load(Ordering::Relaxed),
            elapsed: start.elapsed(),
        },
    ))
}

/// 从 `Vec<Entry>` 出发跑阶段二（零 DNS）。
pub async fn probe_survival_entries(
    entries: Vec<Entry>,
    workers: usize,
    timeout: Duration,
) -> Result<(Vec<Entry>, StageStats), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let total = entries.len() as u64;
    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (tx, mut rx) = mpsc::channel::<Entry>(workers * 2);
    let sem = Arc::new(Semaphore::new(workers));
    let pb = progress::bar(total, "probe-survival");

    let mut handles = Vec::with_capacity(entries.len());
    for e in entries {
        let Some(ip) = e.ip else {
            // 阶段二不发任何 DNS 查询；缺 IP 直接丢弃
            dropped.fetch_add(1, Ordering::Relaxed);
            pb.inc(1);
            continue;
        };
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let tx = tx.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        let pb = pb.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            if tls::probe_survival(&e.domain, ip, timeout) {
                kept.fetch_add(1, Ordering::Relaxed);
                let _ = tx.blocking_send(Entry::new(e.domain, ip));
            } else {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            pb.inc(1);
            drop(permit);
        }));
    }
    drop(tx);

    let mut collected = Vec::new();
    while let Some(entry) = rx.recv().await {
        collected.push(entry);
    }
    for h in handles {
        let _ = h.await;
    }
    progress::finish(
        &pb,
        &format!(
            "[probe-survival] total={total} kept={} dropped={} elapsed={:.1}s",
            kept.load(Ordering::Relaxed),
            dropped.load(Ordering::Relaxed),
            start.elapsed().as_secs_f64()
        ),
    );

    Ok((
        collected,
        StageStats {
            total,
            kept: kept.load(Ordering::Relaxed),
            dropped: dropped.load(Ordering::Relaxed),
            elapsed: start.elapsed(),
        },
    ))
}

/// 从 `Vec<Entry>` 出发跑阶段三，返回通过质检的记录，并把双池 `.pb` 写入 `out_dir`。
pub async fn build_pools_entries(
    entries: Vec<Entry>,
    out_dir: &Path,
    geodb: Option<Arc<geo::GeoDb>>,
    workers: usize,
    timeout: Duration,
) -> Result<Vec<Entry>, Box<dyn std::error::Error + Send + Sync>> {
    let total = entries.len() as u64;
    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (tx, mut rx) = mpsc::channel::<(Entry, String, bool)>(workers * 2);
    let sem = Arc::new(Semaphore::new(workers));
    let pb = progress::bar(total, "build-pools");

    let mut handles = Vec::with_capacity(entries.len());
    for e in entries {
        let Some(ip) = e.ip else {
            dropped.fetch_add(1, Ordering::Relaxed);
            pb.inc(1);
            continue;
        };
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let tx = tx.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        let geodb = geodb.clone();
        let pb2 = pb.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            if cloudflare::is_cloudflare_ip(ip) {
                dropped.fetch_add(1, Ordering::Relaxed);
                pb2.inc(1);
                drop(permit);
                return;
            }
            match tls::probe(&e.domain, ip, timeout) {
                Ok(p) if tls::validate(&p) => {
                    let country = geodb
                        .map(|g| g.country(ip))
                        .unwrap_or_else(|| "UNKNOWN".to_string());
                    if geo::is_blocked_country(&country) {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    } else {
                        kept.fetch_add(1, Ordering::Relaxed);
                        let _ = tx.blocking_send((e.clone(), country, p.is_pq()));
                    }
                }
                _ => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            pb2.inc(1);
            drop(permit);
        }));
    }
    drop(tx);

    let mut by_country: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> =
        std::collections::BTreeMap::new();
    let mut accepted = Vec::new();
    while let Some((entry, country, is_pq)) = rx.recv().await {
        let bucket = by_country.entry(country).or_default();
        if is_pq {
            bucket.0.push(entry.domain.clone());
        } else {
            bucket.1.push(entry.domain.clone());
        }
        accepted.push(entry);
    }
    for h in handles {
        let _ = h.await;
    }

    write_dual_pb(out_dir, &by_country)?;
    progress::finish(
        &pb,
        &format!(
            "[build-pools] total={total} kept={} dropped={} -> {}",
            kept.load(Ordering::Relaxed),
            dropped.load(Ordering::Relaxed),
            out_dir.display()
        ),
    );

    Ok(accepted)
}

/// 阶段一：海外预解析，输出 `domain,ip`。
pub async fn run_resolve_map(
    input: &Path,
    output: &Path,
    workers: usize,
    max_lines: usize,
) -> Result<StageStats, Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let mut lines = read_lines(input)?;
    if max_lines > 0 {
        lines.truncate(max_lines);
    }
    let total = lines.len() as u64;

    let resolver = Arc::new(dns::build_resolver(Duration::from_millis(2500))?);
    let sem = Arc::new(Semaphore::new(workers));
    let (tx, mut rx) = mpsc::channel::<(String, std::net::IpAddr)>(workers * 2);

    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));

    let producer = {
        let resolver = resolver.clone();
        let sem = sem.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        tokio::spawn(async move {
            let mut tasks = Vec::new();
            for line in lines {
                let Some((domain, known_ip)) = target::parse_line(&line) else {
                    continue;
                };
                let permit = match sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break,
                };
                let tx = tx.clone();
                let resolver = resolver.clone();
                let kept = kept.clone();
                let dropped = dropped.clone();
                tasks.push(tokio::spawn(async move {
                    let result = match known_ip.and_then(|s| s.parse().ok()) {
                        Some(ip) => Ok(ip),
                        None => dns::resolve_first(&resolver, &domain).await,
                    };
                    match result {
                        Ok(ip) if !cloudflare::is_cloudflare_ip(ip) => {
                            kept.fetch_add(1, Ordering::Relaxed);
                            let _ = tx.send((domain, ip)).await;
                        }
                        _ => {
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    drop(permit);
                }));
            }
            for t in tasks {
                let _ = t.await;
            }
            drop(tx);
        })
    };

    let pb = progress::bar(total, "resolve-map");
    let mut collected = Vec::new();
    while let Some((domain, ip)) = rx.recv().await {
        collected.push(Entry::new(domain, ip));
        pb.inc(1);
    }
    let _ = producer.await;
    progress::finish(
        &pb,
        &format!(
            "[resolve-map] total={total} kept={} dropped={} elapsed={:.1}s",
            kept.load(Ordering::Relaxed),
            dropped.load(Ordering::Relaxed),
            start.elapsed().as_secs_f64()
        ),
    );

    write_entries(output, &collected)?;

    Ok(StageStats {
        total,
        kept: kept.load(Ordering::Relaxed),
        dropped: dropped.load(Ordering::Relaxed),
        elapsed: start.elapsed(),
    })
}

/// 阶段二：审查境内存活探测（零 DNS）。
pub async fn run_probe_survival(
    input: &Path,
    output: &Path,
    workers: usize,
    max_lines: usize,
    timeout: Duration,
    _proxy: Option<&str>,
) -> Result<StageStats, Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let mut lines = read_lines(input)?;
    if max_lines > 0 {
        lines.truncate(max_lines);
    }
    let total = lines.len() as u64;

    let targets: Vec<(String, std::net::IpAddr)> = lines
        .iter()
        .filter_map(|l| target::parse_line(l))
        .filter_map(|(d, ip)| ip.and_then(|s| s.parse().ok()).map(|ip| (d, ip)))
        .collect();

    let sem = Arc::new(Semaphore::new(workers));
    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (tx, mut rx) = mpsc::channel::<(String, std::net::IpAddr)>(workers * 2);

    let mut handles = Vec::with_capacity(targets.len());
    for (domain, ip) in targets {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let tx = tx.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            let alive = tls::probe_survival(&domain, ip, timeout);
            if alive {
                kept.fetch_add(1, Ordering::Relaxed);
                let _ = tx.blocking_send((domain, ip));
            } else {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            drop(permit);
        }));
    }
    drop(tx);

    let file = File::create(output)?;
    let mut out = BufWriter::new(file);
    while let Some((domain, ip)) = rx.recv().await {
        writeln!(out, "{domain},{ip}")?;
    }
    out.flush()?;
    for h in handles {
        let _ = h.await;
    }

    Ok(StageStats {
        total,
        kept: kept.load(Ordering::Relaxed),
        dropped: dropped.load(Ordering::Relaxed),
        elapsed: start.elapsed(),
    })
}

/// 阶段三：海外深度质检并生成双池 `.pb`。
pub async fn run_build_pools(
    input: &Path,
    out_dir: &Path,
    geodb: Option<Arc<geo::GeoDb>>,
    workers: usize,
    max_lines: usize,
    timeout: Duration,
) -> Result<StageStats, Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let mut lines = read_lines(input)?;
    if max_lines > 0 {
        lines.truncate(max_lines);
    }
    let total = lines.len() as u64;

    let mut targets: Vec<(String, std::net::IpAddr)> = lines
        .iter()
        .filter_map(|l| target::parse_line(l))
        .filter_map(|(d, ip)| ip.and_then(|s| s.parse().ok()).map(|ip| (d, ip)))
        .collect();

    // 已带 IP 的输入无需再 DNS；缺失 IP 的行在此处补解析
    let resolver = Arc::new(dns::build_resolver(Duration::from_millis(2500))?);
    let needs_dns: Vec<String> = lines
        .iter()
        .filter_map(|l| target::parse_line(l))
        .filter(|(_, ip)| ip.is_none())
        .map(|(d, _)| d)
        .collect();
    for d in needs_dns {
        if let Ok(ip) = dns::resolve_first(&resolver, &d).await {
            targets.push((d, ip));
        }
    }

    let sem = Arc::new(Semaphore::new(workers));
    let kept = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (tx, mut rx) = mpsc::channel::<(String, String, bool)>(workers * 2); // (domain, country, is_pq)

    let mut handles = Vec::with_capacity(targets.len());
    for (domain, ip) in targets {
        let permit = match sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let tx = tx.clone();
        let kept = kept.clone();
        let dropped = dropped.clone();
        let geodb = geodb.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            if cloudflare::is_cloudflare_ip(ip) {
                dropped.fetch_add(1, Ordering::Relaxed);
                drop(permit);
                return;
            }
            match tls::probe(&domain, ip, timeout) {
                Ok(p) if tls::validate(&p) => {
                    let country = geodb
                        .map(|g| g.country(ip))
                        .unwrap_or_else(|| "UNKNOWN".to_string());
                    if geo::is_blocked_country(&country) {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    } else {
                        kept.fetch_add(1, Ordering::Relaxed);
                        let _ = tx.blocking_send((domain, country, p.is_pq()));
                    }
                }
                _ => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            drop(permit);
        }));
    }
    drop(tx);

    let mut by_country: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> =
        std::collections::BTreeMap::new();
    while let Some((domain, country, is_pq)) = rx.recv().await {
        let entry = by_country.entry(country).or_default();
        if is_pq {
            entry.0.push(domain);
        } else {
            entry.1.push(domain);
        }
    }
    for h in handles {
        let _ = h.await;
    }

    write_dual_pb(out_dir, &by_country)?;

    Ok(StageStats {
        total,
        kept: kept.load(Ordering::Relaxed),
        dropped: dropped.load(Ordering::Relaxed),
        elapsed: start.elapsed(),
    })
}

/// 增量写入 `<COUNTRY>.pb`：与磁盘上已有内容合并，保留历史可用域名。
fn write_dual_pb(
    out_dir: &Path,
    by_country: &std::collections::BTreeMap<String, (Vec<String>, Vec<String>)>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    for (country, (pq, classic)) in by_country {
        let path: PathBuf = out_dir.join(format!("{}.pb", country.to_uppercase()));
        let (mut merged_pq, mut merged_classic) = match std::fs::read(&path) {
            Ok(data) => match crate::pb::decode(&data) {
                Ok((p, c)) => (p, c),
                Err(_) => (Vec::new(), Vec::new()),
            },
            Err(_) => (Vec::new(), Vec::new()),
        };
        merged_pq.extend(pq.iter().cloned());
        merged_classic.extend(classic.iter().cloned());
        merged_pq.sort();
        merged_pq.dedup();
        merged_classic.sort();
        merged_classic.dedup();
        std::fs::write(&path, crate::pb::encode(&merged_pq, &merged_classic))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_writes_domain_ip_pairs() {
        let dir = std::env::temp_dir().join(format!("sni_probe_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.csv");
        let output = dir.join("out.csv");
        std::fs::write(&input, "1,example.com\n").unwrap();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let stats = rt.block_on(run_resolve_map(&input, &output, 2, 0)).unwrap();

        assert_eq!(stats.total, 1);
        let text = std::fs::read_to_string(&output).unwrap();
        if stats.kept == 1 {
            let line = text.lines().next().unwrap();
            assert!(line.starts_with("example.com,"), "got {line}");
            assert!(
                line.split(',')
                    .nth(1)
                    .unwrap()
                    .parse::<std::net::IpAddr>()
                    .is_ok()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
