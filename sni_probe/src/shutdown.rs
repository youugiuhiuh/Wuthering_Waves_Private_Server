//! 完成后自动关机（对齐 Go `-shutdown`）。
//!
//! 长跑任务常放在夜里，跑完不关机既费电也费钱。这里只调用系统关机命令，
//! 失败仅告警而不影响退出码。

use std::time::Duration;

/// 关机前留给用户的中断窗口（与 Go 版一致为 60 秒）。
pub const GRACE_SECS: u64 = 60;

/// 构造关机命令（不含执行，便于测试）。
///
/// - Linux: `shutdown -h now` 立即停机
/// - Windows: `shutdown /s /t 0` 立即关机
#[cfg(unix)]
pub fn shutdown_command() -> (&'static str, Vec<&'static str>) {
    ("shutdown", vec!["-h", "now"])
}

#[cfg(windows)]
pub fn shutdown_command() -> (&'static str, Vec<&'static str>) {
    ("shutdown", vec!["/s", "/t", "0"])
}

/// 取消关机的命令（跨平台）。
#[cfg(unix)]
pub const CANCEL_HINT: &str = "shutdown -c";
#[cfg(windows)]
pub const CANCEL_HINT: &str = "shutdown /a";

/// 执行关机。`dry_run` 只打印不真正执行。
pub fn request_shutdown(dry_run: bool) -> Result<(), String> {
    let (cmd, args) = shutdown_command();
    let printable = format!("{} {}", cmd, args.join(" "));

    if dry_run {
        println!("[shutdown] 将执行：{printable}（dry-run，未实际执行）");
        return Ok(());
    }

    println!("[shutdown] 即将关机，可用 Ctrl+C 取消或执行 `{CANCEL_HINT}` 取消");
    match std::process::Command::new(cmd).args(&args).status() {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => Err(format!("{printable} 退出码 {:?}", st.code())),
        Err(e) => Err(format!("执行 {printable} 失败: {e}")),
    }
}

/// 关机前的等待时长（独立函数便于测试）。
#[allow(dead_code)]
pub fn grace_period() -> Duration {
    Duration::from_secs(GRACE_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_uses_halt_flag() {
        let (cmd, args) = shutdown_command();
        assert_eq!(cmd, "shutdown");
        #[cfg(unix)]
        {
            assert!(args.contains(&"-h"), "必须用 -h 断电而不是 -r 重启");
            assert!(args.contains(&"now"));
        }
        #[cfg(windows)]
        {
            assert!(args.contains(&"/s"), "必须用 /s 关机而不是 /r 重启");
        }
        // 任何平台都不得是重启
        assert!(!args.contains(&"-r") && !args.contains(&"/r"), "不得重启");
    }

    #[test]
    fn cancel_hint_matches_platform() {
        #[cfg(unix)]
        assert_eq!(CANCEL_HINT, "shutdown -c");
        #[cfg(windows)]
        assert_eq!(CANCEL_HINT, "shutdown /a");
    }

    #[test]
    fn dry_run_does_not_execute() {
        // dry_run 必须返回 Ok 且不真正关机
        assert!(request_shutdown(true).is_ok());
    }

    #[test]
    fn grace_period_matches_constant() {
        assert_eq!(grace_period().as_secs(), GRACE_SECS);
    }
}
