//! 启动时资源自检：防止 `-w` 超过机器承受能力后产生大量**假失败**。
//!
//! 背景：探测任务 99% 的时间在等网络 RTT，所以直觉上「并发越高越快」。
//! 但并发受两件事约束：
//!
//! - **文件描述符**：每个进行中的探测占 1 个 socket。超过 `ulimit -n` 后
//!   新连接直接 `Too many open files`，这些失败会被写进历史库，
//!   下次运行还会被当成「近期失败」跳过——污染会累积。
//! - **CPU 核心数**：单核机器上 `-w 500` 时，TLS 解密 + 密钥协商互相抢占，
//!   每个探测的 CPU 时间被拉长，整体吞吐反而下降。

use std::fmt;

/// 一台机器上可安全使用的并发上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConcurrencyBudget {
    /// 系统当前的 soft `RLIMIT_NOFILE`
    pub fd_soft_limit: u64,
    /// 可用 CPU 核心数
    pub cpu_cores: u64,
    /// 由 FD 推出的上限
    pub fd_bound: u64,
    /// 由 CPU 推出的上限
    pub cpu_bound: u64,
}

impl ConcurrencyBudget {
    /// 推荐并发值（取两个约束的较小者）。
    pub fn recommended(&self) -> u64 {
        self.fd_bound.min(self.cpu_bound).max(1)
    }

    /// 判定 `requested` 是否超出安全范围。
    pub fn verdict(&self, requested: u64) -> Verdict {
        let rec = self.recommended();
        if requested <= rec {
            Verdict::Ok
        } else if requested <= rec * 2 {
            // 两倍以内：能跑，但会开始有零星 EMFILE / CPU 抢占
            Verdict::Risky
        } else {
            Verdict::TooHigh
        }
    }
}

/// 资源检查结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 在安全范围内
    Ok,
    /// 超过推荐值但不到两倍：建议下调
    Risky,
    /// 远超推荐值：会大量产生假失败
    TooHigh,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Ok => write!(f, "OK"),
            Verdict::Risky => write!(f, "偏高"),
            Verdict::TooHigh => write!(f, "过高"),
        }
    }
}

/// 系统给定的 FD 里，留给非探测用途的余量。
///
/// 进程本身、历史库、DNS socket、日志等都要占 FD，
/// 另外留一点给内核瞬时分配的临时句柄。
const FD_RESERVE_ABS: u64 = 128;
/// 除固定预留外，再按比例预留 10%
const FD_RESERVE_RATIO: f64 = 0.10;

/// 每个 CPU 核心建议容纳的并发探测数。
///
/// 单次握手的 CPU 成本约 4ms（BoringSSL + X25519/Kyber），
/// 而一次探测的墙钟时间约 600ms～1s；比值约 150，
/// 保守取 20 留出充裕裕量，同时避免单核上过度抢占。
const PER_CORE: u64 = 20;

/// 读取当前进程的 soft `RLIMIT_NOFILE`。
pub fn fd_soft_limit() -> u64 {
    // 不能用 `ulimit` shell 命令：那会新起进程，且拿到的是 shell 的限制。
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit 只写我们传入的结构体，指针有效且独占。
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
    if rc != 0 {
        return 1024; // 取保守默认
    }
    lim.rlim_cur
}

/// 可用 CPU 核心数。
pub fn cpu_cores() -> u64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(1)
}

/// 根据实测资源推算安全并发。
pub fn budget() -> ConcurrencyBudget {
    budget_from(fd_soft_limit(), cpu_cores())
}

/// 纯函数版本，便于测试。
pub fn budget_from(fd_soft: u64, cores: u64) -> ConcurrencyBudget {
    let reserve = FD_RESERVE_ABS + (fd_soft as f64 * FD_RESERVE_RATIO) as u64;
    let fd_bound = fd_soft.saturating_sub(reserve).max(1);
    let cpu_bound = cores.max(1) * PER_CORE;

    ConcurrencyBudget {
        fd_soft_limit: fd_soft,
        cpu_cores: cores.max(1),
        fd_bound,
        cpu_bound,
    }
}

/// 生成给用户看的告警文本；`Ok` 时返回 `None`。
pub fn warn_text(b: &ConcurrencyBudget, requested: u64, stage: &str) -> Option<String> {
    match b.verdict(requested) {
        Verdict::Ok => None,
        v => {
            let mut s = String::new();
            s.push_str(&format!(
                "\n⚠ 并发数 -w {requested} 超出本机安全范围（判定：{v}）\n"
            ));
            s.push_str(&format!(
                "   实测：ulimit -n = {}, CPU 核心 = {}\n",
                b.fd_soft_limit, b.cpu_cores
            ));
            // 明确写出「谁在卡上限」，否则读者会误以为推荐值没考虑更紧的那一侧
            let (tight, loose, tight_name) = if b.fd_bound < b.cpu_bound {
                (b.fd_bound, b.cpu_bound, "FD")
            } else {
                (b.cpu_bound, b.fd_bound, "CPU")
            };
            s.push_str(&format!(
                "   约束：FD 上限 {}，CPU 上限 {} → 受 {tight_name} 限制，推荐 -w {tight}\n",
                b.fd_bound, b.cpu_bound
            ));
            let _ = loose;
            if b.fd_bound < b.cpu_bound {
                // 用 concat! 而非续行，避免 \ 续行把前导缩进吃掉
                s.push_str(concat!(
                    "   瓶颈是文件描述符：超限会造成 `Too many open files`，\n",
                    "   这些失败会被写入历史库，下次运行还会被当成「近期失败」跳过。\n",
                    "   解决：`ulimit -n 65535` 或调低 -w。\n",
                ));
            } else {
                s.push_str(concat!(
                    "   瓶颈是 CPU：单次握手约 4ms CPU，并发过高会互相抢占，\n",
                    "   耗时被拉长、超时增多——这些超时同样会被写入历史库，\n",
                    "   下次运行被当成「近期失败」跳过。\n",
                    "   解决：调低 -w 或换更多核的机器。\n",
                ));
            }
            s.push_str(&format!("   [{stage}] 建议改用：-w {}\n", b.recommended()));
            // 两种瓶颈都会污染历史库，统一在末尾再点一次，避免被分支吞掉
            if !s.contains("历史库") {
                s.push_str(concat!(
                    "   注：超限导致的失败会被写入历史库，下次运行按「近期失败」跳过，\n",
                    "   污染会累积，必要时用 --reset 清空。\n",
                ));
            }
            Some(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_core_low_ulimit_gives_small_budget() {
        // 复现实测到的那台服务器：1 核 + soft limit 1024
        let b = budget_from(1024, 1);
        assert_eq!(b.cpu_bound, 20);
        // 1024 - (128 + 102) = 794
        assert_eq!(b.fd_bound, 794);
        assert_eq!(b.recommended(), 20, "CPU 是瓶颈");
    }

    #[test]
    fn high_ulimit_multi_core_scales_with_cores() {
        let b = budget_from(524288, 8);
        assert_eq!(b.cpu_bound, 160);
        assert_eq!(b.recommended(), 160);
    }

    #[test]
    fn fd_bound_dominates_when_ulimit_is_tiny() {
        let b = budget_from(256, 8);
        // 256 - (128 + 25) = 103
        assert_eq!(b.fd_bound, 103);
        assert_eq!(b.recommended(), 103, "FD 是瓶颈");
    }

    #[test]
    fn zero_or_tiny_limits_never_yield_zero() {
        assert!(budget_from(0, 1).recommended() >= 1);
        assert!(budget_from(1, 1).recommended() >= 1);
        assert!(budget_from(1024, 0).recommended() >= 1, "0 核也不得返回 0");
    }

    #[test]
    fn verdict_thresholds() {
        let b = budget_from(1024, 1); // rec = 20
        assert_eq!(b.verdict(1), Verdict::Ok);
        assert_eq!(b.verdict(20), Verdict::Ok, "等于推荐值算安全");
        assert_eq!(b.verdict(21), Verdict::Risky);
        assert_eq!(b.verdict(40), Verdict::Risky, "正好两倍仍算接近");
        assert_eq!(b.verdict(41), Verdict::TooHigh);
        assert_eq!(b.verdict(500), Verdict::TooHigh, "用户误填 500 的场景");
    }

    #[test]
    fn no_warning_when_within_budget() {
        let b = budget_from(1024, 1);
        assert!(warn_text(&b, 10, "build-pools").is_none());
        assert!(warn_text(&b, 20, "build-pools").is_none());
    }

    #[test]
    fn warning_names_the_binding_constraint() {
        // 单核：CPU 是瓶颈
        let cpu_b = budget_from(524288, 1);
        let w = warn_text(&cpu_b, 500, "build-pools").expect("应告警");
        assert!(w.contains("瓶颈是 CPU"), "单核应指出 CPU 瓶颈:\n{w}");
        assert!(w.contains("-w 20"), "应给出推荐值:\n{w}");

        // 小 FD：FD 是瓶颈
        let fd_b = budget_from(256, 8);
        let w2 = warn_text(&fd_b, 500, "resolve-map").expect("应告警");
        assert!(w2.contains("瓶颈是文件描述符"), "应指出 FD 瓶颈:\n{w2}");
        assert!(w2.contains("ulimit -n"), "应给出解决办法:\n{w2}");
    }

    #[test]
    fn warning_mentions_history_pollution_risk() {
        let b = budget_from(1024, 1);
        let w = warn_text(&b, 500, "build-pools").expect("应告警");
        assert!(w.contains("历史库"), "应提醒假失败会污染历史库:\n{w}");
    }

    /// 回归：Rust 的 `\` 续行会吞掉下一行前导空白，
    /// 曾导致告警缩进全丢，读起来像一堆散句。
    #[test]
    fn warning_lines_keep_indentation() {
        let b = budget_from(1024, 1);
        let w = warn_text(&b, 500, "build-pools").expect("应告警");
        let body: Vec<&str> = w
            .lines()
            .filter(|l| l.contains("历史库") || l.contains("解决："))
            .collect();
        assert!(!body.is_empty());
        for line in body {
            assert!(
                line.starts_with("   "),
                "告警正文行必须保留缩进，实际: {line:?}"
            );
        }
    }

    /// 回归：曾出现「FD 上限 794、CPU 上限 480，推荐 480」却配文字
    /// 「FD 允许约 794」的情况，读者会误以为推荐值忽略了更紧的 FD。
    /// 现在必须明写受哪一侧限制。
    #[test]
    fn warning_states_which_side_binds() {
        let b = budget_from(1024, 24); // fd=794, cpu=480 -> CPU 更紧
        assert_eq!(b.recommended(), 480);
        let w = warn_text(&b, 500, "x").expect("应告警");
        assert!(w.contains("受 CPU 限制"), "应写明受 CPU 限制:\n{w}");
        assert!(!w.contains("受 FD 限制"), "不应误报 FD:\n{w}");

        let b2 = budget_from(256, 8); // fd=103, cpu=160 -> FD 更紧
        let w2 = warn_text(&b2, 500, "x").expect("应告警");
        assert!(w2.contains("受 FD 限制"), "应写明受 FD 限制:\n{w2}");
    }

    #[test]
    fn live_probe_returns_sane_values() {
        // 真读本机 getrlimit / 核数，不得 panic 或返回 0
        let fd = fd_soft_limit();
        assert!(fd > 0, "ulimit -n 读取失败");
        let cores = cpu_cores();
        assert!(cores >= 1);
        let b = budget();
        assert!(b.recommended() >= 1);
    }
}
