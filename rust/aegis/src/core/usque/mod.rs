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

pub use engine::{Egress, select_egress};
pub use installer::{InstalledKinds, UninstallOutcome};
