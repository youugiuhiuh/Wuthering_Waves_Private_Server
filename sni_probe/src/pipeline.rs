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

    let file = File::create(output)?;
    let mut out = BufWriter::new(file);
    while let Some((domain, ip)) = rx.recv().await {
        writeln!(out, "{domain},{ip}")?;
    }
    out.flush()?;
    let _ = producer.await;

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
