//! 出口引擎的选择与缓存。
//!
//! # 选择是「派生的」，不是「存储的」
//!
//! 出口用哪套实现，完全由 [`ProbeConclusion`] 推出（见 [`select_egress`]），
//! 不引入新的持久化状态。这避免了「配置里写的是 MASQUE，实际探测结果是
//! WireGuard 可用」这类不一致。
//!
//! # 为什么要缓存
//!
//! 探测是**网络 I/O**（一个 UDP 包 + 一次 TLS 握手，超时 3s）。
//! 它出现在 Xray 配置生成路径上，而那条路径会被菜单渲染、规则增删等
//! 高频操作触发。每次现探会给这些操作加上最多 3s 延迟，
//! 并引入一个与本操作无关的新失败面。
//!
//! 故缓存 [`ProbeReport`]，TTL 见 [`CACHE_TTL`]。
//!
//! # 与 Spec 1 的差别（刻意的）
//!
//! Spec 1 的诊断按钮**不缓存**——那是管理员主动点击的实时诊断。
//! 这里**必须缓存**——它是配置生成的输入，要的是确定性与低延迟。
//! 同一份探测数据，两种用途，两种缓存策略，互不矛盾。
//!
//! # 全部为纯逻辑 + 可注入时钟
//!
//! 时间通过参数传入而非读系统时钟，故 TTL 行为可确定性单测。

use crate::core::network::{ProbeConclusion, ProbeReport};
use std::sync::Mutex;
use std::time::Duration;

/// 探测结果缓存有效期。
///
/// 10 分钟：够长以避免高频配置生成反复发探测包，
/// 够短以跟随网络条件变化（例如机房策略变更后能在 10 分钟内切换出口）。
pub const CACHE_TTL: Duration = Duration::from_secs(600);

/// 出口引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Egress {
    /// Xray 内嵌 `proxy/wireguard`（UDP 2408）。
    WireGuard,
    /// usque 提供的本地 SOCKS（MASQUE over TCP/443）。
    Masque,
    /// 不经 WARP，走 `direct` 出口。
    Direct,
}

impl Egress {
    /// 是否启用 WARP 出口。
    ///
    /// `Direct` 表示**不启用**——两条通路都不可用时宁可直连，
    /// 也不要把用户流量指向一个收不到回包的黑洞。
    pub fn is_warp_enabled(&self) -> bool {
        !matches!(self, Egress::Direct)
    }

    /// 供日志/状态展示用的稳定标识。
    pub fn as_str(&self) -> &'static str {
        match self {
            Egress::WireGuard => "wireguard",
            Egress::Masque => "masque",
            Egress::Direct => "direct",
        }
    }
}

/// 由探测结论选择出口引擎。
///
/// 映射依据：
///
/// | 结论 | 出口 | 理由 |
/// |---|---|---|
/// | `WgUsable` | WireGuard | UDP 通 → 用开销更低的原生 WireGuard |
/// | `UdpOnly` | WireGuard | 同上（TCP 不通与 WireGuard 无关） |
/// | `TcpOnly` | Masque | UDP 被封、TCP 可用 → 这正是本模块存在的理由 |
/// | `FullyBlocked` | Direct | 两条都不可用 → 不启用 WARP，避免黑洞 |
pub fn select_egress(conclusion: ProbeConclusion) -> Egress {
    match conclusion {
        ProbeConclusion::WgUsable => Egress::WireGuard,
        ProbeConclusion::UdpOnly => Egress::WireGuard,
        ProbeConclusion::TcpOnly => Egress::Masque,
        ProbeConclusion::FullyBlocked => Egress::Direct,
    }
}

/// 带时间戳的缓存项。
#[derive(Debug, Clone, Copy)]
struct Cached {
    report: ProbeReport,
    at: Duration,
}

/// 探测结果缓存（进程内）。
#[derive(Debug, Default)]
pub struct ProbeCache {
    inner: Mutex<Option<Cached>>,
}

impl ProbeCache {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// 存入一份探测结果。`now` 为当前单调时刻。
    pub fn store(&self, report: ProbeReport, now: Duration) {
        if let Ok(mut g) = self.inner.lock() {
            *g = Some(Cached { report, at: now });
        }
    }

    /// 读取未过期的缓存；未探测或已过期返回 `None`。
    pub fn get(&self, now: Duration, ttl: Duration) -> Option<ProbeReport> {
        let g = self.inner.lock().ok()?;
        let c = (*g)?;
        // 时钟回拨（now < at）视为过期，而不是「非常新鲜」。
        if now.checked_sub(c.at).is_some_and(|age| age < ttl) {
            Some(c.report)
        } else {
            None
        }
    }

    /// 清空缓存（供 `uninstall` / 强制刷新使用）。
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            *g = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::network::{TcpVerdict, UdpVerdict};

    fn report(udp: UdpVerdict, tcp: TcpVerdict) -> ProbeReport {
        ProbeReport {
            udp,
            tcp,
            conclusion: ProbeConclusion::from_verdicts(udp, tcp),
        }
    }

    /// **穷举 4 态**——不是抽两个。
    #[test]
    fn test_select_egress_exhaustive() {
        use ProbeConclusion::*;
        assert_eq!(select_egress(WgUsable), Egress::WireGuard);
        assert_eq!(select_egress(UdpOnly), Egress::WireGuard);
        assert_eq!(select_egress(TcpOnly), Egress::Masque);
        assert_eq!(select_egress(FullyBlocked), Egress::Direct);
    }

    /// `FullyBlocked` **不得**启用 WARP。
    ///
    /// 这是防黑洞的关键：两条通路都不可用时若仍指向 WARP，
    /// 全部用户流量会静默超时。
    #[test]
    fn test_fully_blocked_disables_warp() {
        let e = select_egress(ProbeConclusion::FullyBlocked);
        assert_eq!(e, Egress::Direct);
        assert!(!e.is_warp_enabled(), "出口完全受限时必须禁用 WARP");
    }

    #[test]
    fn test_warp_enabled_for_usable_engines() {
        assert!(Egress::WireGuard.is_warp_enabled());
        assert!(Egress::Masque.is_warp_enabled());
        assert!(!Egress::Direct.is_warp_enabled());
    }

    /// 本部署的实际处境（UDP 被封、TCP 可用）必须选 MASQUE。
    /// 这条把 `FINDINGS.md` 的真机结论固化成可执行断言。
    #[test]
    fn test_this_deployments_condition_selects_masque() {
        let r = report(UdpVerdict::Blocked, TcpVerdict::AvailableH2);
        assert_eq!(r.conclusion, ProbeConclusion::TcpOnly);
        assert_eq!(select_egress(r.conclusion), Egress::Masque);
    }

    /// 本地发包失败（`ProbeFailed`）不得被当成「UDP 可用」。
    #[test]
    fn test_probe_failed_does_not_mean_udp_usable() {
        // 发包失败 + TCP 无 h2 → 完全受限
        let r = report(UdpVerdict::ProbeFailed, TcpVerdict::OpenNoH2);
        assert_eq!(select_egress(r.conclusion), Egress::Direct);
        // 发包失败 + TCP 可用 → 走 MASQUE
        let r = report(UdpVerdict::ProbeFailed, TcpVerdict::AvailableH2);
        assert_eq!(select_egress(r.conclusion), Egress::Masque);
    }

    #[test]
    fn test_egress_as_str_is_stable() {
        assert_eq!(Egress::WireGuard.as_str(), "wireguard");
        assert_eq!(Egress::Masque.as_str(), "masque");
        assert_eq!(Egress::Direct.as_str(), "direct");
    }

    // ───────────────────────── 缓存 ─────────────────────────

    #[test]
    fn test_cache_miss_when_empty() {
        let c = ProbeCache::new();
        assert!(c.get(Duration::from_secs(0), CACHE_TTL).is_none());
    }

    #[test]
    fn test_cache_hit_within_ttl() {
        let c = ProbeCache::new();
        let r = report(UdpVerdict::Blocked, TcpVerdict::AvailableH2);
        c.store(r, Duration::from_secs(100));
        // 100s 存入，109s 读取 → 9s 前，未过期
        let got = c.get(Duration::from_secs(109), CACHE_TTL).expect("应命中");
        assert_eq!(got, r);
    }

    #[test]
    fn test_cache_expires_after_ttl() {
        let c = ProbeCache::new();
        let r = report(UdpVerdict::Blocked, TcpVerdict::AvailableH2);
        c.store(r, Duration::from_secs(0));
        // 恰好 TTL → 视为过期（用 < 而非 <=）
        assert!(c.get(CACHE_TTL, CACHE_TTL).is_none(), "恰好 TTL 应过期");
        assert!(
            c.get(CACHE_TTL + Duration::from_secs(1), CACHE_TTL)
                .is_none(),
            "超过 TTL 应过期"
        );
        // TTL 前一瞬仍有效
        assert!(
            c.get(CACHE_TTL - Duration::from_secs(1), CACHE_TTL)
                .is_some()
        );
    }

    #[test]
    fn test_cache_clear() {
        let c = ProbeCache::new();
        c.store(
            report(UdpVerdict::Available, TcpVerdict::AvailableH2),
            Duration::from_secs(0),
        );
        assert!(c.get(Duration::from_secs(1), CACHE_TTL).is_some());
        c.clear();
        assert!(c.get(Duration::from_secs(1), CACHE_TTL).is_none());
    }

    #[test]
    fn test_cache_overwrite() {
        let c = ProbeCache::new();
        let a = report(UdpVerdict::Available, TcpVerdict::AvailableH2);
        let b = report(UdpVerdict::Blocked, TcpVerdict::AvailableH2);
        c.store(a, Duration::from_secs(0));
        c.store(b, Duration::from_secs(1));
        assert_eq!(c.get(Duration::from_secs(2), CACHE_TTL), Some(b));
    }

    /// 时钟回拨必须视为过期，而不是「非常新鲜」。
    ///
    /// 单调时钟正常不会回拨，但用 `Duration` 参数注入时调用方可能传错；
    /// 宁可在那种情况下重新探测，也不要拿着可能陈旧的数据当真。
    #[test]
    fn test_cache_treats_clock_rewind_as_expired() {
        let c = ProbeCache::new();
        c.store(
            report(UdpVerdict::Available, TcpVerdict::AvailableH2),
            Duration::from_secs(1000),
        );
        // 读取时刻早于存入时刻
        assert!(
            c.get(Duration::from_secs(10), CACHE_TTL).is_none(),
            "回拨应判过期"
        );
    }

    #[test]
    fn test_cache_ttl_is_ten_minutes() {
        assert_eq!(CACHE_TTL, Duration::from_secs(600));
    }
}
