use crate::core::error::{AppError, Result};
use crate::core::types::IpVersion;
use rand::{RngExt, rngs::StdRng};
use std::time::{Duration, Instant};

pub const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_secs(2);
pub const PROGRESS_PERCENT_STEP: f64 = 5.0;
pub const PROGRESS_SIZE_STEP: u64 = 5 * 1024 * 1024;

/// 归一化版本标签：去首尾空白与**单个**前导 `v`。
///
/// 上游 Release tag 形如 `v26.9.30`，而核心自报版本为 `26.9.30`，两者需归一后才能比较。
/// 只剥一层 `v`：`vv1.2.3` 保留第二个 `v`，避免误吞版本内容。
pub fn normalize_version_tag(tag: &str) -> &str {
    tag.trim().strip_prefix('v').unwrap_or(tag.trim())
}

/// 判断本地核心版本与远端 tag 是否为同一版本。
///
/// 刻意**只做相等判断**：不引入大小比较。上游存在预发行通道（sing-box 走
/// `fetch_prerelease`），大小语义由上游 tag 决定，bot 侧不应自行判定「本地更新」
/// 而拒绝管理员显式指定的升级。
pub fn is_same_version(local: &str, remote_tag: &str) -> bool {
    let local = normalize_version_tag(local);
    let remote = normalize_version_tag(remote_tag);
    !local.is_empty() && local == remote
}

/// 通用端口选择
pub async fn select_available_port(preferred: Option<u16>) -> Result<u16> {
    if let Some(port) = preferred
        && crate::core::system::maintenance::MaintenanceManager::is_port_available(port).await
    {
        return Ok(port);
    }

    let mut rng = rand::make_rng::<StdRng>();
    for _ in 0..1000 {
        let port = rng.random_range(10000..60000);
        if crate::core::system::maintenance::MaintenanceManager::is_port_available(port).await {
            return Ok(port);
        }
    }

    Err(AppError::PortUnavailable(0))
}

/// 生成随机字符串后缀
pub fn generate_random_suffix(length: usize) -> String {
    let mut rng = rand::make_rng::<StdRng>();
    let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..length)
        .map(|_| {
            let idx = rng.random_range(0..chars.len());
            chars[idx] as char
        })
        .collect()
}

/// 生成时间戳+随机后缀的文件名
pub fn generate_timestamp_filename(prefix: &str, extension: &str) -> String {
    let timestamp = chrono::Utc::now().timestamp();
    let suffix = generate_random_suffix(8);
    format!("{}_{}_{}.{}", prefix, timestamp, suffix, extension)
}

/// 安全地解析 IP 版本字符串
pub fn parse_ip_version(s: &str) -> Option<IpVersion> {
    match s {
        "4" | "ipv4" | "IPv4" => Some(IpVersion::IPv4),
        "6" | "ipv6" | "IPv6" => Some(IpVersion::IPv6),
        "split6" => Some(IpVersion::SplitStackV6Primary),
        "split4" => Some(IpVersion::SplitStackV4Primary),
        _ => None,
    }
}

/// 格式化字节大小为可读字符串
pub fn human_readable_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

/// 格式化下载进度文本
pub fn format_download_progress(downloaded: u64, total: Option<u64>, start: Instant) -> String {
    let elapsed = start.elapsed().as_secs_f64().max(0.1);
    let speed = downloaded as f64 / elapsed;
    let speed_human = human_readable_size(speed as u64) + "/s";
    match total {
        Some(total_size) => {
            let pct = downloaded as f64 * 100.0 / total_size as f64;
            format!(
                "📥 下载中... {}/{} ({:.1}%)\n⚡ 速度: {}",
                human_readable_size(downloaded),
                human_readable_size(total_size),
                pct,
                speed_human
            )
        }
        None => format!(
            "📥 下载中... {} (总大小未知)\n⚡ 速度: {}",
            human_readable_size(downloaded),
            speed_human
        ),
    }
}

/// 判断是否应该汇报进度以避免 Telegram 消息频率限制
pub fn should_report(
    downloaded: u64,
    total: Option<u64>,
    last_pct: &mut f64,
    last_size: &mut u64,
    last_instant: Instant,
) -> bool {
    let elapsed = last_instant.elapsed();
    let size_diff = downloaded - *last_size;
    let pct = total
        .map(|t| downloaded as f64 * 100.0 / t as f64)
        .unwrap_or(0.0);

    if let Some(total_size) = total {
        if pct >= *last_pct + PROGRESS_PERCENT_STEP {
            *last_pct = pct;
            *last_size = downloaded;
            return true;
        }
        if downloaded == total_size && pct >= 99.0 {
            return true;
        }
    }

    if size_diff >= PROGRESS_SIZE_STEP {
        *last_size = downloaded;
        return true;
    }

    elapsed >= PROGRESS_UPDATE_INTERVAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_version_tag_strips_prefix_and_space() {
        assert_eq!(normalize_version_tag("v1.2.3"), "1.2.3");
        assert_eq!(normalize_version_tag("  v1.2.3  "), "1.2.3");
        assert_eq!(normalize_version_tag("1.2.3"), "1.2.3");
        // 仅去一个前缀：``vv1`` 保留第二个 ``v``，避免误吞版本内容。
        assert_eq!(normalize_version_tag("vv1.2.3"), "v1.2.3");
        assert_eq!(normalize_version_tag(""), "");
    }

    #[test]
    fn test_is_same_version_table() {
        let cases = [
            ("26.9.30", "v26.9.30", true),
            ("v26.9.30", "26.9.30", true),
            (" 26.9.30 ", " v26.9.30 ", true),
            ("26.9.30", "v26.9.31", false),
            ("1.8.4", "v1.8.4-rc1", false),
            ("1.8.4-rc1", "v1.8.4", false),
            ("", "v1.2.3", false),
            ("1.2.3", "", false),
            ("", "", false),
        ];

        for (local, remote, expected) in cases {
            assert_eq!(
                is_same_version(local, remote),
                expected,
                "local={local:?} remote={remote:?}"
            );
        }
    }

    #[test]
    fn test_generate_random_suffix_length() {
        assert_eq!(generate_random_suffix(0).len(), 0);
        assert_eq!(generate_random_suffix(5).len(), 5);
        assert_eq!(generate_random_suffix(16).len(), 16);
    }

    #[test]
    fn test_generate_random_suffix_characters() {
        let suffix = generate_random_suffix(1000);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn test_generate_random_suffix_deterministic() {
        let s1 = generate_random_suffix(8);
        let s2 = generate_random_suffix(8);
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_generate_timestamp_filename_format() {
        let filename = generate_timestamp_filename("config", "json");
        let parts: Vec<&str> = filename.split('_').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "config");
        assert!(parts[1].parse::<i64>().is_ok());
        assert!(parts[2].ends_with(".json"));
    }

    #[test]
    fn test_generate_timestamp_filename_extension_without_dot() {
        let filename = generate_timestamp_filename("test", "log");
        assert!(filename.ends_with(".log"));
    }

    #[test]
    fn test_parse_ip_version_ipv4_variants() {
        assert_eq!(parse_ip_version("4"), Some(IpVersion::IPv4));
        assert_eq!(parse_ip_version("ipv4"), Some(IpVersion::IPv4));
        assert_eq!(parse_ip_version("IPv4"), Some(IpVersion::IPv4));
    }

    #[test]
    fn test_parse_ip_version_ipv6_variants() {
        assert_eq!(parse_ip_version("6"), Some(IpVersion::IPv6));
        assert_eq!(parse_ip_version("ipv6"), Some(IpVersion::IPv6));
        assert_eq!(parse_ip_version("IPv6"), Some(IpVersion::IPv6));
    }

    #[test]
    fn test_parse_ip_version_split_stack() {
        assert_eq!(
            parse_ip_version("split6"),
            Some(IpVersion::SplitStackV6Primary)
        );
        assert_eq!(
            parse_ip_version("split4"),
            Some(IpVersion::SplitStackV4Primary)
        );
    }

    #[test]
    fn test_parse_ip_version_invalid() {
        assert_eq!(parse_ip_version(""), None);
        assert_eq!(parse_ip_version("invalid"), None);
        assert_eq!(parse_ip_version("ipv7"), None);
        assert_eq!(parse_ip_version("v4"), None);
        assert_eq!(parse_ip_version("v6"), None);
    }

    #[test]
    fn test_human_readable_size() {
        assert_eq!(human_readable_size(0), "0 B");
        assert_eq!(human_readable_size(512), "512 B");
        assert_eq!(human_readable_size(1024), "1.00 KB");
        assert_eq!(human_readable_size(1024 * 1024), "1.00 MB");
        assert_eq!(human_readable_size(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(
            human_readable_size(2 * 1024 * 1024 * 1024 * 1024),
            "2.00 TB"
        );
    }

    #[test]
    fn test_format_download_progress_with_total() {
        let start = Instant::now() - Duration::from_secs(1);
        let text = format_download_progress(1024 * 1024, Some(2 * 1024 * 1024), start);
        assert!(text.contains("50.0%"));
        assert!(text.contains("速度"));
        assert!(text.contains("/s"));
    }

    #[test]
    fn test_format_download_progress_unknown_total() {
        let start = Instant::now() - Duration::from_secs(1);
        let text = format_download_progress(1024 * 1024, None, start);
        assert!(text.contains("总大小未知"));
        assert!(text.contains("速度"));
    }

    #[test]
    fn test_should_report_triggers_on_percent_step() {
        let mut last_pct = 0.0;
        let mut last_size = 0u64;
        let last_instant = Instant::now() - Duration::from_secs(3);

        assert!(should_report(
            5 * 1024 * 1024,
            Some(100 * 1024 * 1024),
            &mut last_pct,
            &mut last_size,
            last_instant
        ));
        assert_eq!(last_pct, 5.0);
    }

    #[test]
    fn test_should_report_triggers_on_size_step() {
        let mut last_pct = 0.0;
        let mut last_size = 0u64;
        let last_instant = Instant::now() - Duration::from_secs(3);

        assert!(should_report(
            5 * 1024 * 1024,
            None,
            &mut last_pct,
            &mut last_size,
            last_instant
        ));
    }

    #[test]
    fn test_should_report_triggers_on_completion() {
        let mut last_pct = 0.0;
        let mut last_size = 0u64;
        let last_instant = Instant::now() - Duration::from_secs(3);

        assert!(should_report(
            100,
            Some(100),
            &mut last_pct,
            &mut last_size,
            last_instant
        ));
    }

    #[test]
    fn test_should_report_no_trigger_within_interval() {
        let mut last_pct = 0.0;
        let mut last_size = 0u64;
        let last_instant = Instant::now();

        assert!(!should_report(
            1024,
            Some(1024 * 1024),
            &mut last_pct,
            &mut last_size,
            last_instant
        ));
    }
}
