//! usque —— WARP over TCP/443（MASQUE / HTTP-2）。
//!
//! # 为什么需要这个模块
//!
//! Xray 的 `proxy/wireguard` 是 **UDP-only**（WireGuard 协议本身不支持跑在
//! TCP 上）。当机房在出站方向丢弃非 53 端口的 UDP 时，WireGuard 路径永远
//! 建不起来：`wg0` 的 `rx_bytes` 恒为 0、`tx_bytes` 持续增长，经 WARP 的
//! 流量全部静默超时（真机证据见 `tasks/usque-tcp/FINDINGS.md`）。
//!
//! usque 是 Cloudflare WARP 的 MASQUE 开源实现，可走 **TCP/443**。
//! 它不是一个库，而是一个**独立进程**：
//!
//! ```text
//! Xray [socks outbound] ──> 127.0.0.1:1080 ──> usque ──TCP 443/H2──> Cloudflare
//! ```
//!
//! # 模块划分
//!
//! - [`pin`]：版本与 sha256 的**硬编码 pin**，以及相关纯函数（URL 构造、校验、解析）
//! - [`unit`]：systemd unit 文本生成（纯函数，可断言）
//! - [`engine`]：由探测结论选择出口引擎（纯函数）+ 内存缓存
//! - [`health`]：隧道健康判据与失败计数（纯函数）
//! - [`installer`]：I/O —— 下载、校验、注册、安装、卸载、状态探测

pub mod engine;
pub mod health;
pub mod installer;
pub mod pin;
pub mod unit;

pub use engine::{Egress, now_monotonic, select_egress};
pub use installer::{InstallPlan, InstalledKinds, UninstallOutcome, install_plan};

/// 启动 usque 的后台任务。
///
/// # 只 spawn 健康检查循环——**不** spawn 定时探测
///
/// 这是用户确认的决策（W3）：探测**只在安装时跑一次**。
/// 理由：出口条件（机房是否封 UDP）是**机房级策略**，不随时间漂移。
/// 本部署实测 UDP 2408 被丢、TCP 443 通，这是固定的。
///
/// 缓存 TTL（10min）到期后 `current_egress()` 会回退 `Direct`，
/// 但那**只在重新生成 Xray 配置时**被读到，且回退 `Direct`
/// **不会黑洞**——`build_routing_rules` 在 `enforce=false` 时
/// 根本不生成 catch-all。想重新启用，管理员再点一次安装即可（幂等）。
///
/// 健康检查则**必须有**：上游已知 usque 会卡死但进程不退出，
/// `Restart=always` 对它无效（见 [`health::health_loop`]）。
///
/// # 为何封成单一入口
///
/// `main/runtime.rs` 有 **3 处**启动点（TG / SimpleX / 组合）。
/// 封成一个函数能让三处接入方式一致，避免某个部署形态下漏接——
/// 那种缺失是完全静默的。
pub fn spawn_background_tasks() {
    tokio::spawn(async move {
        health::health_loop().await;
    });
}

#[cfg(test)]
mod tests {
    /// **防回归**：后台任务只能有健康检查，不得加回定时探测。
    ///
    /// 这是用户确认的决策 W3。若后人「顺手」把定时重探加回来，
    /// 会引入一个常驻循环 + 一份「探测失败时是否降级」的新判断面，
    /// 而它换来的收益（跟随机房策略变化）在本部署并不存在。
    ///
    /// 断言源码而非运行时：`spawn` 数量在运行时不可观测。
    #[test]
    fn test_only_health_loop_is_spawned_in_background() {
        const SRC: &str = include_str!("mod.rs");

        let start = SRC
            .find("pub fn spawn_background_tasks()")
            .expect("spawn_background_tasks 必须存在");
        let body = &SRC[start..];
        let body = &body[..body.find("\n}").unwrap_or(body.len())];

        assert!(body.contains("health_loop"), "后台任务必须包含健康检查循环");
        assert_eq!(
            body.matches("tokio::spawn").count(),
            1,
            "后台只应有 1 个 spawn（健康检查）。\
             若新增了定时探测，请先重新确认决策 W3：\
             出口条件是机房级策略、不随时间漂移，\
             且缓存回退 Direct 不会黑洞（enforce=false 时不生成 catch-all）。"
        );
        assert!(
            !body.contains("run_probe"),
            "后台不得包含定时探测——探测只在安装时跑一次（决策 W3）"
        );
    }
}
