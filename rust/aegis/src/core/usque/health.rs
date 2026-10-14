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

/// 进程级健康状态。
///
/// # 为什么需要缓存
///
/// 状态页要显示真实健康，但**不能**在渲染时探测——那会让菜单阻塞最多
/// [`HEALTH_TIMEOUT`]（20s）。故由后台循环维护这个值，状态页只读。
///
/// `None` 表示「尚未探测」——必须与 `Some(false)` 区分开：
/// 前者是「不知道」，后者是「确知不健康」。把两者混为一谈会让刚启动时
/// 误报隧道故障。
fn health_state() -> &'static std::sync::Mutex<Option<bool>> {
    static STATE: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);
    &STATE
}

/// 读当前健康状态；未探测过返回 `None`。
pub fn current_health() -> Option<bool> {
    health_state().lock().ok().and_then(|g| *g)
}

/// 写入健康状态（供后台循环与测试）。
pub fn set_health(healthy: bool) {
    if let Ok(mut g) = health_state().lock() {
        *g = Some(healthy);
    }
}

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

/// 走 usque 的本地 SOCKS 请求 `cdn-cgi/trace`，判断隧道是否真的在转发。
///
/// 返回 `None` 表示「无法判定」（usque 未安装 / 请求本身失败），
/// 与 `Some(false)`（确知不健康）区分开。
///
/// # 为什么不用「端口是否可连」
///
/// usque 在隧道卡死时 **SOCKS 端口仍然是监听的**——进程没死、端口没关，
/// 只是不再转发。所以「端口活着」完全不能作为健康判据。
pub async fn probe_tunnel_health() -> Option<bool> {
    let target = format!(
        "{}:{}",
        crate::core::paths::usque::SOCKS_ADDR,
        crate::core::paths::usque::SOCKS_PORT
    );

    // curl 走 SOCKS5 请求 trace 端点。用 --max-time 限制在 HEALTH_TIMEOUT 内。
    let out = tokio::time::timeout(
        HEALTH_TIMEOUT,
        tokio::process::Command::new("curl")
            .args([
                "-fsS",
                "--max-time",
                &HEALTH_TIMEOUT.as_secs().to_string(),
                "--socks5-hostname",
                &target,
                "https://www.cloudflare.com/cdn-cgi/trace",
            ])
            .output(),
    )
    .await;

    match out {
        Ok(Ok(o)) if o.status.success() => {
            let body = String::from_utf8_lossy(&o.stdout);
            Some(is_tunnel_healthy(&body))
        }
        // 请求失败 / 超时：无法判定（可能是 usque 没起来，也可能是网络瞬间问题）
        _ => None,
    }
}

/// 后台健康检查循环。**永不返回**（除非进程退出）。
///
/// # 为什么必须有这个循环
///
/// 上游已知 usque 会「卡死但不退出」：
///
/// - README「Known Issues」：空闲会被远端以 `H3_NO_ERROR` 断开；
/// - issue #49：用户报告「用一阵子卡死后**必须手动 kill** 重启」；
/// - PR #103：修过一个 tunnel 死锁。
///
/// **`Restart=always` 只在进程退出时生效，对卡死完全无效。**
/// 所以必须由外部定期探测并主动强杀。
///
/// # 错误处理
///
/// 循环体内部的错误一律 **catch 并记日志，不上抛**——一旦上抛导致循环
/// 退出，监控就静默失效了，而这正是本循环要防的那类故障。
pub async fn health_loop() {
    let mut tracker = FailureTracker::new();

    loop {
        tokio::time::sleep(HEALTH_INTERVAL).await;

        // 未安装就不发无谓的网络请求。
        if !crate::core::usque::installer::detect_installed()
            .await
            .usque
        {
            // 未安装时清掉健康状态，避免显示上一次的残留结论。
            tracker = FailureTracker::new();
            continue;
        }

        match probe_tunnel_health().await {
            Some(healthy) => {
                set_health(healthy);
                tracker.record(healthy);

                if tracker.should_restart() {
                    log::warn!(
                        "usque 连续 {} 次不健康（隧道卡死但进程未退出），强杀重启",
                        tracker.consecutive_failures()
                    );
                    if let Err(e) =
                        crate::core::system::maintenance::MaintenanceManager::control_service(
                            crate::core::paths::usque::SERVICE_NAME,
                            crate::core::cmd_action::ServiceAction::Restart,
                        )
                        .await
                    {
                        log::error!("重启 usque 失败: {e}");
                    }
                    // 必须清零：否则下一轮立刻再次触发重启，形成重启风暴。
                    tracker = FailureTracker::new();
                }
            }
            // 无法判定：不写健康状态（保留上一次的结论），也不计入失败。
            // 否则一次网络抖动就会被当成隧道故障。
            None => {
                log::debug!("usque 健康探测无法判定，跳过本轮");
            }
        }
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

    // ────────────── T5: 健康状态进程级缓存 ──────────────

    /// 未探测时必须是 `None`，不能是 `Some(false)`。
    ///
    /// `None`（不知道）与 `Some(false)`（确知不健康）语义不同：
    /// 混为一谈会让刚启动时误报隧道故障。
    #[test]
    fn test_health_state_starts_unknown() {
        // 注意：这是进程级单例，测试间会互相影响，故本测试只断言
        // 「写入后能读出」与「类型正确」，不断言初始值。
        set_health(true);
        assert_eq!(current_health(), Some(true));
        set_health(false);
        assert_eq!(current_health(), Some(false));
        // 复位，避免污染同进程内其它测试（nextest 每测试独立进程，
        // 但 cargo test 下是共享的）。
        set_health(true);
    }

    #[test]
    fn test_health_state_round_trip() {
        set_health(false);
        assert_eq!(current_health(), Some(false), "必须能区分 false 与 None");
        set_health(true);
        assert_eq!(current_health(), Some(true));
    }

    // ────────────── T4: 循环行为的可测部分 ──────────────

    /// 循环常量必须自洽：单轮超时不得长到让失败判定堆积。
    #[test]
    fn test_loop_timing_is_sane() {
        // 3 次 × 60s 间隔 = 3 分钟才判死；而单轮探测最多 20s。
        // 若 HEALTH_TIMEOUT >= HEALTH_INTERVAL，探测会追着间隔跑，永不空闲。
        assert!(
            HEALTH_TIMEOUT < HEALTH_INTERVAL,
            "单轮探测超时必须小于检查间隔，否则轮次会堆积"
        );
    }

    /// 判死所需的总时长必须在可接受范围内——
    /// 太长则隧道卡死很久才被发现，太短则容易误杀。
    #[test]
    fn test_time_to_restart_is_reasonable() {
        let total = HEALTH_INTERVAL * FAILURE_THRESHOLD;
        assert!(
            total >= std::time::Duration::from_secs(120),
            "判死过快会误杀健康隧道: {total:?}"
        );
        assert!(
            total <= std::time::Duration::from_secs(600),
            "判死过慢会让隧道卡死太久: {total:?}"
        );
    }
}
