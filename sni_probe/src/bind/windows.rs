//! Windows 平台实现：只支持**按本地 IP 绑定**。
//!
//! Windows 没有 `SO_BINDTODEVICE` 的等价物：
//! - `IP_UNICAST_IF` 只影响组播/特定场景，且要按接口**索引**而非名字
//! - `bind()` 到本地 IP 是跨平台的标准做法，效果等价（同 IP 必然同网卡）
//!
//! 因此本平台 `--iface` 只接受 IP 字面量；给网卡名由 `mod.rs` 拒绝。

#![cfg(windows)]

use std::net::{IpAddr, SocketAddr};

use super::BindTarget;

/// Windows 没有等价的按网卡绑定，恒返回 `None`。
pub fn iface_ipv4(_name: &str) -> Option<IpAddr> {
    None
}

/// Windows 不支持按网卡名枚举（`--list-ifaces` 由 mod.rs 返回空）。
pub fn list_interfaces() -> Vec<String> {
    Vec::new()
}

/// 创建 socket 并绑定到指定本地 IP，再连接到目标。
///
/// `std` 没有「带源地址 bind 的 connect_timeout」组合，这里用 `socket2`：
/// 它是 tokio 已经带进来的传递依赖，跨平台一致，且不用手写 WinSock FFI
/// （手写版在 windows-sys 0.61 上踩了太多类型/命名差异）。
pub fn connect_bound(
    target: SocketAddr,
    timeout: std::time::Duration,
    bind: &BindTarget,
) -> std::io::Result<std::net::TcpStream> {
    let Some(local_ip) = bind.local_ip else {
        return std::net::TcpStream::connect_timeout(&target, timeout);
    };

    let local = SocketAddr::new(local_ip, 0);
    if local.is_ipv4() != target.is_ipv4() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("源地址 {local_ip} 与目标 {target} 地址族不一致"),
        ));
    }

    let domain = if local.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let sock = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    // bind() 到本地 IP：走系统路由表，效果等价于按网卡出站
    sock.bind(&local.into())?;
    sock.connect_timeout(&target.into(), timeout)?;
    Ok(sock.into())
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

    /// 源地址族与目标不一致必须在建 socket 之前被拒绝。
    #[test]
    fn mismatched_family_rejected() {
        let bind = BindTarget {
            iface: None,
            local_ip: Some("127.0.0.1".parse().unwrap()),
        };
        let target: SocketAddr = "[::1]:443".parse().unwrap();
        let err = connect_bound(target, std::time::Duration::from_millis(1), &bind).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// 绑定本机回环地址连本机监听器，验证 bind+connect 真实可用。
    #[test]
    fn bound_loopback_connects() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap();
        let bind = BindTarget {
            iface: None,
            local_ip: Some("127.0.0.1".parse().unwrap()),
        };
        let stream = connect_bound(target, std::time::Duration::from_secs(1), &bind).unwrap();
        assert_eq!(stream.local_addr().unwrap().ip(), target.ip());
    }
}
