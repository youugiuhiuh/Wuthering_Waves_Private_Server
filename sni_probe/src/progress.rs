//! 进度条：无 TTY 时自动退化为纯文本/静默，避免污染日志。

use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

/// 有总量的进度条（阶段一/二已知输入行数）。
///
/// 非 TTY 环境下返回隐藏进度条，调用方仍需正常 `inc()`/`set_position()`。
pub fn bar(total: u64, label: &str) -> ProgressBar {
    if !is_tty() {
        let pb = ProgressBar::hidden();
        pb.set_length(total);
        return pb;
    }

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {prefix:.bold} [{bar:32.cyan/blue}] {pos}/{len} ({percent}%) {per_sec} ETA {eta}",
        )
        .expect("内置模板必须合法")
        .progress_chars("=> "),
    );
    pb.set_prefix(label.to_string());
    pb.enable_steady_tick(Duration::from_millis(200));
    pb
}

/// 完成收尾：清除动画并打印一行汇总。
pub fn finish(pb: &ProgressBar, summary: &str) {
    if !pb.is_hidden() {
        pb.finish_and_clear();
    }
    println!("{summary}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_tracks_total_and_position() {
        let pb = bar(10, "test");
        pb.set_position(5);
        assert_eq!(pb.position(), 5);
        if !pb.is_hidden() {
            assert_eq!(pb.length(), Some(10));
        }
        finish(&pb, "done");
    }

    #[test]
    fn bar_handles_zero_total() {
        let pb = bar(0, "empty");
        if !pb.is_hidden() {
            assert_eq!(pb.length(), Some(0));
        }
        finish(&pb, "done");
    }
}
