//! Windows 平台实现：只支持**按本地 IP 绑定**。
//!
//! Windows 没有 `SO_BINDTODEVICE` 的等价物：
//! - `IP_UNICAST_IF` 只影响组播/特定场景，且要按接口**索引**而非名字
//! - `bind()` 到本地 IP 是跨平台的标准做法，效果等价（同 IP 必然同网卡）
//!
//! 因此本平台 `--iface` 只接受 IP 字面量；给网卡名由 `mod.rs` 拒绝。

#![cfg(windows)]

use std::net::{IpAddr, SocketAddr};
use std::os::windows::io::AsRawSocket;

use super::BindTarget;

/// Windows 没有等价的按网卡绑定，恒返回 `None`。
pub fn iface_ipv4(_name: &str) -> Option<IpAddr> {
    None
}

/// Windows 不支持按网卡名枚举（`--list-ifaces` 由 mod.rs 返回空）。
pub fn list_interfaces() -> Vec<String> {
    Vec::new()
}

/// 创建 socket 并 `bind()` 到指定本地 IP，再连接到目标。
///
/// 用 `std::net::TcpStream::bind()`（Rust 1.70+ 稳定）完成源地址绑定，
/// 避免手写 WinSock FFI。
pub fn connect_bound(
    target: SocketAddr,
    timeout: std::time::Duration,
    bind: &BindTarget,
) -> std::io::Result<std::net::TcpStream> {
    let Some(local_ip) = bind.local_ip else {
        return std::net::TcpStream::connect_timeout(&target, timeout);
    };

    // std 的 bind 只接受同族地址：IPv4 目标绑 IPv4 源，IPv6 目标绑 IPv6 源
    let local = SocketAddr::new(local_ip, 0);
    let socket = if local.is_ipv4() == target.is_ipv4() {
        std::net::TcpSocket::new_v4().or_else(|_| std::net::TcpSocket::new_v6())?
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("源地址 {local_ip} 与目标 {target} 地址族不一致"),
        ));
    };

    socket.bind(local)?;
    let _ = socket.as_raw_socket(); // 仅用于触发 import lint
    socket.set_read_timeout(Some(timeout)).ok();
    socket.set_write_timeout(Some(timeout)).ok();
    socket.connect_timeout(target, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reports_no_iface_support() {
        assert!(!super::super::IFACE_BIND_SUPPORTED);
        assert!(list_interfaces().is_empty());
        assert!(iface_ipv4("Ethernet").is_none());
    }
}
