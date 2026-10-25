//! 跨文件 `.pb` 合并去重（对齐 Go `pkg/protobuf.go` 的
//! `LoadExistingBinFiles` / `DedupeSorted`）。
//!
//! 场景：`build-pools` 每次只合并同名国家文件，但磁盘上可能残留
//! 不同时期的产物（如 `US.pb` 与备份目录里的 `US.pb`），需要先汇总去重，
//! 避免同一个域名在池里出现多次。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::pb;
use crate::upgrade::PbFormat;

/// 排除的国家文件：审查管辖地，与 `geo::is_blocked_country` 保持一致。
const EXCLUDED_FILES: &[&str] = &["CN.PB", "HK.PB", "MO.PB", "IR.PB", "RU.PB", "KP.PB"];

/// 排序去重并剔除空串（等价 Go `DedupeSorted`）。
pub fn dedupe_sorted(list: &mut Vec<String>) {
    list.retain(|d| !d.is_empty());
    list.sort();
    list.dedup();
}

/// 把多个列表合并成有序无重复的一个。
#[allow(dead_code)] // 工具函数：供需要跨列表并集的调用方使用
pub fn merge_lists(lists: &[Vec<String>]) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for l in lists {
        for d in l {
            if !d.is_empty() {
                set.insert(d.clone());
            }
        }
    }
    set.into_iter().collect()
}

/// 收集目录下全部 `.pb` 的域名（不含被排除的国家）。
///
/// 返回 `(pq, classic)`，两池各自去重。
pub fn collect_dir(
    dir: &Path,
    format: PbFormat,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error + Send + Sync>> {
    let mut pq: Vec<String> = Vec::new();
    let mut classic: Vec<String> = Vec::new();
    let mut files = pb_files(dir)?;

    // 稳定排序：让输出可预测，便于比对与测试
    files.sort();

    for f in files {
        if is_excluded(&f) {
            continue;
        }
        let data = match std::fs::read(&f) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[merge] 跳过 {}: {e}", f.display());
                continue;
            }
        };
        match crate::upgrade::decode_as(&data, format) {
            Ok((p, c)) => {
                pq.extend(p);
                classic.extend(c);
            }
            Err(e) => eprintln!("[merge] 解析 {} 失败: {e}", f.display()),
        }
    }

    dedupe_sorted(&mut pq);
    dedupe_sorted(&mut classic);
    // 同一域名不得同时出现在两池：体温以 PQ 为准
    let pq_set: BTreeSet<&String> = pq.iter().collect();
    classic.retain(|d| !pq_set.contains(d));

    Ok((pq, classic))
}

fn pb_files(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pb")) {
            out.push(p);
        }
    }
    Ok(out)
}

fn is_excluded(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_uppercase();
    EXCLUDED_FILES.contains(&name.as_str())
}

/// 把目录内全部 `.pb` 合并后写回单个文件（用于产出总池）。
pub fn merge_dir_into(
    dir: &Path,
    out: &Path,
    format: PbFormat,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    let (pq, classic) = collect_dir(dir, format)?;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, pb::encode(&pq, &classic))?;
    Ok((pq.len(), classic.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("merge_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dedupe_removes_empty_and_duplicates() {
        let mut v = vec![
            "b.com".to_string(),
            "".to_string(),
            "a.com".to_string(),
            "b.com".to_string(),
            "".to_string(),
        ];
        dedupe_sorted(&mut v);
        assert_eq!(v, vec!["a.com", "b.com"]);
    }

    #[test]
    fn merge_lists_unions_without_duplicates() {
        let merged = merge_lists(&[
            vec!["a.com".into(), "b.com".into()],
            vec!["b.com".into(), "c.com".into()],
        ]);
        assert_eq!(merged, vec!["a.com", "b.com", "c.com"]);
    }

    #[test]
    fn empty_inputs_yield_empty_output() {
        let mut v: Vec<String> = Vec::new();
        dedupe_sorted(&mut v);
        assert!(v.is_empty());
        assert!(merge_lists(&[]).is_empty());
        assert!(merge_lists(&[vec![], vec![]]).is_empty());
    }

    #[test]
    fn collects_and_dedupes_across_files() {
        let dir = tmp("collect");
        // AA 与 BB 有重复域名
        std::fs::write(
            dir.join("AA.pb"),
            pb::encode(&["a.com".into(), "shared.com".into()], &["x.com".into()]),
        )
        .unwrap();
        std::fs::write(
            dir.join("BB.pb"),
            pb::encode(&["b.com".into(), "shared.com".into()], &["x.com".into()]),
        )
        .unwrap();

        let (pq, classic) = collect_dir(&dir, PbFormat::DualPool).unwrap();
        assert_eq!(pq, vec!["a.com", "b.com", "shared.com"], "PQ 应跨文件去重");
        assert_eq!(classic, vec!["x.com"], "classic 也应去重");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 同一域名同时出现在两池时，必须只保留在 PQ 池。
    #[test]
    fn pq_wins_when_domain_in_both_pools() {
        let dir = tmp("pqwins");
        std::fs::write(
            dir.join("AA.pb"),
            pb::encode(&["dup.com".into()], &["dup.com".into()]),
        )
        .unwrap();

        let (pq, classic) = collect_dir(&dir, PbFormat::DualPool).unwrap();
        assert_eq!(pq, vec!["dup.com"]);
        assert!(classic.is_empty(), "不得同时留在 classic 池");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn excludes_censoring_jurisdiction_files() {
        let dir = tmp("exclude");
        std::fs::write(dir.join("US.pb"), pb::encode(&["us.com".into()], &[])).unwrap();
        std::fs::write(dir.join("CN.pb"), pb::encode(&["cn.com".into()], &[])).unwrap();
        std::fs::write(dir.join("RU.pb"), pb::encode(&["ru.com".into()], &[])).unwrap();
        std::fs::write(dir.join("IR.pb"), pb::encode(&["ir.com".into()], &[])).unwrap();
        std::fs::write(dir.join("kp.PB"), pb::encode(&["kp.com".into()], &[])).unwrap();

        let (pq, _) = collect_dir(&dir, PbFormat::DualPool).unwrap();
        assert_eq!(pq, vec!["us.com"], "管辖国文件必须跳过（含大小写混写）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_non_pb_files() {
        let dir = tmp("nonpb");
        std::fs::write(dir.join("US.pb"), pb::encode(&["us.com".into()], &[])).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a pb").unwrap();
        std::fs::write(dir.join("readme.md"), b"x").unwrap();

        let (pq, _) = collect_dir(&dir, PbFormat::DualPool).unwrap();
        assert_eq!(pq, vec!["us.com"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_format_puts_everything_in_classic() {
        let dir = tmp("legacy");
        // 旧单池：只有 field 1
        std::fs::write(dir.join("OLD.pb"), pb::encode(&["old.com".into()], &[])).unwrap();

        let (pq, classic) = collect_dir(&dir, PbFormat::LegacySinglePool).unwrap();
        assert!(pq.is_empty(), "旧格式不得进 PQ 池");
        assert_eq!(classic, vec!["old.com"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_dir_into_writes_single_file() {
        let dir = tmp("into");
        let src = dir.join("sni");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("AA.pb"),
            pb::encode(&["a.com".into()], &["c.com".into()]),
        )
        .unwrap();
        std::fs::write(src.join("BB.pb"), pb::encode(&["b.com".into()], &[])).unwrap();

        let out = dir.join("MERGED.pb");
        let (npq, nclassic) = merge_dir_into(&src, &out, PbFormat::DualPool).unwrap();
        assert_eq!((npq, nclassic), (2, 1));

        let data = std::fs::read(&out).unwrap();
        let (pq, classic) = pb::decode(&data).unwrap();
        assert_eq!(pq, vec!["a.com", "b.com"]);
        assert_eq!(classic, vec!["c.com"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_an_error() {
        assert!(collect_dir(Path::new("/nonexistent/xyz"), PbFormat::DualPool).is_err());
    }
}
