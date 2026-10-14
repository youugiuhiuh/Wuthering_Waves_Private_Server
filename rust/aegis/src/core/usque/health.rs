//! usque 隧道健康判据与失败计数（纯函数）。
//!
//! # 为什么需要「业务层」健康检查
//!
//! 项目在 `core/system/core_health.rs` 里已经写下过这条教训：
//!
//! > 这两道防线保证的是「核心能起来且不崩」，**不是「功能正常」**。
//! > 后者需要业务侧探测才能覆盖。
//!
//! 对 usque 而言，`systemctl is-active` 只说明**进程在跑**，
//! 完全不能说明**隧道在转发流量**。上游已知：
//!
//! - 空闲会被远端以 `H3_NO_ERROR` 断开（README「Known Issues」）；
//! - issue #49：用户报告隧道「用一阵子卡死，**必须手动 kill 重启**」；
//! - PR #103：修过一个 tunnel 死锁。
//!
//! **`Restart=always` 只在进程退出时生效，对「卡死但不退出」完全无效。**
//!
//! 因此判据必须建立在实际流量上：经隧道请求 Cloudflare 的
//! `cdn-cgi/trace`，断言响应含 `warp=on`。
//!
//! # 判据为什么不是「端口活着」
//!
//! usque 的 SOCKS 端口在隧道卡死时**仍然是监听的**——进程没死，
//! 端口没关，只是不再转发。所以「端口可连」不能作为健康判据。

/// 连续失败达到该次数即判定需要强杀重启。
///
/// 取 3 而非 1：单次失败可能只是网络抖动或 Cloudflare 侧瞬时不可用，
/// 立刻重启会引入不必要的抖动。3 次 × [`HEALTH_INTERVAL`] = 3 分钟
/// 的持续不可用，才是真的坏了。
pub const FAILURE_THRESHOLD: u32 = 3;

/// 健康检查间隔。
pub const HEALTH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// 单次健康探测的超时。
///
/// 比 Spec 1 的 `PROBE_TIMEOUT`（3s）宽：这里经完整隧道请求真实
/// HTTPS 站点，首次连接可能包含重连开销。
pub const HEALTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// 判定 `cdn-cgi/trace` 的响应体是否表示隧道工作正常。
///
/// `warp=on` 与 `warp=plus` 都算健康——两者都表示流量确实经过了
/// WARP 网络，区别只是账号等级。
///
/// 判定刻意宽松：逐行找 `warp=` 前缀并比较值，不依赖行的顺序
/// （Cloudflare 的 trace 输出顺序不保证稳定），也容忍空白。
pub fn is_tunnel_healthy(trace_body: &str) -> bool {
    for line in trace_body.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("warp=") {
            let v = v.trim();
            if v == "on" || v == "plus" {
                return true;
            }
        }
    }
    false
}

/// 连续失败计数器。
///
/// 只有「连续」失败才触发重启：一次成功会清零。
/// 否则偶发失败会累积，最终在隧道其实还活着的时候误杀。
#[derive(Debug, Default, Clone, Copy)]
pub struct FailureTracker {
    consecutive: u32,
}

impl FailureTracker {
    pub const fn new() -> Self {
        Self { consecutive: 0 }
    }

    /// 记录一次探测结果。成功清零，失败累加。
    ///
    /// `FailureTracker` 是值语义，`Clone + Copy` 便于测试；
    /// 生产用它时需包在 `Mutex`/`AtomicU32` 里共享。
    pub fn record(&mut self, success: bool) {
        if success {
            self.consecutive = 0;
        } else {
            self.consecutive = self.consecutive.saturating_add(1);
        }
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive
    }

    /// 是否应强杀重启。
    pub fn should_restart(&self) -> bool {
        self.consecutive >= FAILURE_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实 `cdn-cgi/trace` 输出（真机实测，2026-10-14）。
    const REAL_HEALTHY_TRACE: &str = "\
fl=114f321
h=www.cloudflare.com
ip=2a09:bac5:cad3:3228::4ff:3f
ts=1754930575.858
visit_scheme=https
uag=curl/8.15.0
colo=SEA
sliver=none
http=http/2
loc=US
tls=TLSv1.3
sni=plaintext
warp=on
gateway=off
";

    #[test]
    fn test_healthy_on_real_trace() {
        assert!(is_tunnel_healthy(REAL_HEALTHY_TRACE));
    }

    /// WARP+ 也算健康——两者都表示流量走了 WARP 网络。
    #[test]
    fn test_warp_plus_counts_as_healthy() {
        assert!(is_tunnel_healthy("ip=1.2.3.4\nwarp=plus\ncolo=SEA\n"));
    }

    /// 直连（未走 WARP）必须判为不健康——这是本函数的核心用途。
    #[test]
    fn test_warp_off_is_unhealthy() {
        let direct = REAL_HEALTHY_TRACE.replace("warp=on", "warp=off");
        assert!(
            !is_tunnel_healthy(&direct),
            "warp=off 表示流量没走 WARP，必须判不健康"
        );
    }

    /// 不依赖行顺序（Cloudflare 输出顺序不保证）。
    #[test]
    fn test_order_independent() {
        assert!(is_tunnel_healthy("warp=on\nip=1.2.3.4\ncolo=SEA\n"));
        assert!(is_tunnel_healthy("colo=SEA\nwarp=on\n"));
        assert!(is_tunnel_healthy("warp=on"));
    }

    /// 容忍空白与 CR（HTTP 响应可能带 \r\n）。
    #[test]
    fn test_tolerates_whitespace_and_crlf() {
        assert!(is_tunnel_healthy("  warp=on  \n"));
        assert!(is_tunnel_healthy("warp=on\r\ncolo=SEA\r\n"));
        assert!(is_tunnel_healthy("\n\nwarp=on\n\n"));
    }

    #[test]
    fn test_empty_and_garbage_are_unhealthy() {
        assert!(!is_tunnel_healthy(""));
        assert!(!is_tunnel_healthy("   \n  \n"));
        assert!(!is_tunnel_healthy("ip=1.2.3.4\ncolo=SEA\n"));
        assert!(!is_tunnel_healthy("<html>502 Bad Gateway</html>"));
    }

    /// 形近但不同的值不得误判为健康。
    #[test]
    fn test_near_miss_values_are_unhealthy() {
        assert!(!is_tunnel_healthy("warp=off\n"));
        assert!(!is_tunnel_healthy("warp=\n"));
        assert!(!is_tunnel_healthy("warp=maybe\n"));
        // 不是 warp 键
        assert!(!is_tunnel_healthy("gateway=off\n"));
        assert!(!is_tunnel_healthy("notwarp=on\n"));
        // 前缀正确但值只是包含 on
        assert!(!is_tunnel_healthy("warp=online\n"));
    }

    // ─────────────────── FailureTracker ───────────────────

    #[test]
    fn test_first_failure_does_not_trigger() {
        let mut t = FailureTracker::new();
        t.record(false);
        assert_eq!(t.consecutive_failures(), 1);
        assert!(!t.should_restart(), "单次失败可能只是抖动，不应重启");
    }

    #[test]
    fn test_threshold_triggers_restart() {
        let mut t = FailureTracker::new();
        t.record(false);
        t.record(false);
        assert!(!t.should_restart(), "两次仍未达阈值");
        t.record(false);
        assert_eq!(t.consecutive_failures(), FAILURE_THRESHOLD);
        assert!(t.should_restart(), "达到阈值必须重启");
    }

    /// 一次成功必须清零，否则偶发失败累积后会误杀健康隧道。
    #[test]
    fn test_success_resets_counter() {
        let mut t = FailureTracker::new();
        t.record(false);
        t.record(false);
        t.record(true);
        assert_eq!(t.consecutive_failures(), 0, "成功必须清零");
        assert!(!t.should_restart());
        // 清零后再连续失败仍需重新数满
        t.record(false);
        t.record(false);
        assert!(!t.should_restart(), "清零后不应立刻触发");
    }

    /// 阈值要能扛住「大成功小失败」的抖动模式。
    #[test]
    fn test_flapping_never_triggers() {
        let mut t = FailureTracker::new();
        for _ in 0..20 {
            t.record(false);
            t.record(true);
        }
        assert!(
            !t.should_restart(),
            "交替成功失败不应触发重启（隧道其实活着"
        );
    }

    /// 持续失败即使远超阈值也只判定为需要重启（不 panic、不溢出。
    #[test]
    fn test_sustained_failure_is_stable() {
        let mut t = FailureTracker::new();
        for _ in 0..1000 {
            t.record(false);
        }
        assert!(t.should_restart());
        assert_eq!(t.consecutive_failures(), 1000);
    }

    #[test]
    fn test_constants_are_sane() {
        assert_eq!(FAILURE_THRESHOLD, 3);
        assert_eq!(HEALTH_INTERVAL, std::time::Duration::from_secs(60));
        assert_eq!(HEALTH_TIMEOUT, std::time::Duration::from_secs(20));
        // 超时必须远小于检查间隔的倍数，避免探测堆积
        assert!(HEALTH_TIMEOUT < HEALTH_INTERVAL * FAILURE_THRESHOLD);
    }
}
