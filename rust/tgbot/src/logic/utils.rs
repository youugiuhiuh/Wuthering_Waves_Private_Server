use std::time::{Duration, Instant};

pub const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_secs(2);
pub const PROGRESS_PERCENT_STEP: f64 = 5.0;
pub const PROGRESS_SIZE_STEP: u64 = 5 * 1024 * 1024; // 5MB

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
    fn test_human_readable_size() {
        assert_eq!(human_readable_size(512), "512 B");
        assert_eq!(human_readable_size(1024), "1.00 KB");
        assert_eq!(human_readable_size(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn test_format_download_progress() {
        let start = Instant::now() - Duration::from_secs(1);
        let text = format_download_progress(1024 * 1024, Some(2 * 1024 * 1024), start);
        assert!(text.contains("50.0%"));
        assert!(text.contains("速度"));
    }
}
