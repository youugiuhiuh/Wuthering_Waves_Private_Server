/// 检查是否存在调试器附加在当前进程上
/// 如果检测到调试器，将直接强行退出进程或触发销毁模式
pub fn check_debugger() {
    #[cfg(target_os = "linux")]
    {
        use log::warn;
        use std::fs;
        use std::process;
        use std::time::Instant;
        // 1. 耗时检测 (Timing Check)
        // 核心理念: 正常的少量运算在现代 CPU (哪怕是廉价 VPS) 上也应该在极短时间内完成 (通常小于 1ms)。
        // 如果黑客在这个阶段用 gdb 下了断点，程序会被物理挂起等待他敲键盘。
        // 等他按 `c` (continue) 让程序继续走时，时间差就会暴增到几百甚至几千毫秒。
        // 这招完全合法，不会被任何 KVM 宿主机的安全策略拦截。
        let start = Instant::now();
        let mut _x: u64 = 0;
        for i in 0..10_000 {
            _x = _x.wrapping_add(i);
            // 这里我们使用 std::hint::black_box 来告诉编译器：“不要把这个毫无意义的循环优化掉”
            // 注意: 在 Rust 1.66 稳定版稳定了 black_box，如果你的 Rust 版本较老，可能会编译报错。
            // 鉴于我们项目用的是较新版本，这里可以直接使用。
            std::hint::black_box(_x);
        }
        let elapsed = start.elapsed();

        // 我们放宽到 100 毫秒 (0.1秒)，这对于 1 万次简单加法来说极其宽裕。
        // 哪怕此时 VPS 的 CPU 被邻居抢占到了 100% 满载，这段非阻塞的极其简单的存算环也不至于卡 0.1 秒。
        // 如果真超过了 0.1 秒，99.9% 的概率是被人拿调试器断下了。
        if elapsed.as_millis() > 100 {
            warn!("❌ 严重安全警告: 检测到执行时间异常 (可能被调试器挂起)");
            process::exit(1);
        }

        // 2. 读取 /proc/self/status 的 TracerPid
        // 说明: 移除了 ptrace(PTRACE_TRACEME) 以兼容 Docker/LXC 和严格的 VPS 内核环境
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("TracerPid:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() > 1
                        && let Ok(tracer_pid) = parts[1].parse::<i32>()
                            && tracer_pid != 0 {
                                warn!(
                                    "❌ 严重安全警告: 检测到调试器附加 (Tracer PID: {})",
                                    tracer_pid
                                );
                                process::exit(1);
                            }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_debugger_no_panic() {
        check_debugger();
    }

    #[test]
    fn test_tracerpid_parsing() {
        let status = "Name: test\nTracerPid:\t0\nPid: 1234\n";
        let mut tracer_pid: i32 = -1;
        for line in status.lines() {
            if line.starts_with("TracerPid:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    tracer_pid = parts[1].parse::<i32>().unwrap_or(-1);
                }
                break;
            }
        }
        assert_eq!(tracer_pid, 0);
    }

    #[test]
    fn test_tracerpid_detected() {
        let status = "Name: test\nTracerPid:\t12345\nPid: 1234\n";
        let mut tracer_pid: i32 = -1;
        for line in status.lines() {
            if line.starts_with("TracerPid:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    tracer_pid = parts[1].parse::<i32>().unwrap_or(-1);
                }
                break;
            }
        }
        assert_eq!(tracer_pid, 12345);
    }
}
