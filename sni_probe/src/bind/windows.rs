//! Windows 平台实现：只支持**按本地 IP 绑定**。
//!
//! Windows 没有 `SO_BINDTODEVICE` 的等价物：
//! - `IP_UNICAST_IF` 只影响组播/特定场景，且要按接口**索引**而非名字
//! - `bind()` 到本地 IP 是跨平台的标准做法，效果等价（同 IP 必然同网卡）
//!
//! 因此本平台 `--iface` 只接受 IP 字面量；给网卡名由 `mod.rs` 拒绝。

#![cfg(windows)]

use std::net::{IpAddr, SocketAddr};
use std::os::windows::io::FromRawSocket;

use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IPPROTO_TCP, SOCK_STREAM, SOCKADDR, SOCKADDR_IN,
    SOCKADDR_IN6, SOCKET, WSAGetLastError, closesocket, connect, htons, socket,
};

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
/// `std` 没有「带源地址 bind 的 connect_timeout」组合，所以这里用最小的
/// WinSock FFI 建 socket + `bind()` + `connect()`，再把所有权交给
/// `TcpStream::from_raw_socket`。
pub fn connect_bound(
    target: SocketAddr,
    _timeout: std::time::Duration,
    bind: &BindTarget,
) -> std::io::Result<std::net::TcpStream> {
    let Some(local_ip) = bind.local_ip else {
        return std::net::TcpStream::connect_timeout(&target, _timeout);
    };

    // bind() 只接受同族地址：IPv4 目标绑 IPv4 源，IPv6 目标绑 IPv6 源
    let local = SocketAddr::new(local_ip, 0);
    if local.is_ipv4() != target.is_ipv4() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("源地址 {local_ip} 与目标 {target} 地址族不一致"),
        ));
    }

    // windows-sys 0.61：AF_* 是 u16，socket() 要 i32；SOCKET 是 usize（不是 u64）
    let family = if local.is_ipv4() { AF_INET } else { AF_INET6 };
    // SAFETY: socket() 返回原始 socket 或 INVALID_SOCKET。
    let raw: SOCKET = unsafe { socket(family as i32, SOCK_STREAM, IPPROTO_TCP) };
    if raw == 0 || raw == usize::MAX {
        // SAFETY: 上一次 socket 调用刚失败，取线程本地 WinSock 错误码。
        return Err(std::io::Error::from_raw_os_error(unsafe {
            WSAGetLastError()
        }));
    }

    // 从这里开始 socket 归我们管，任何失败都要 closesocket
    let close_on_err = |raw: SOCKET, e: std::io::Error| -> std::io::Error {
        // SAFETY: raw 有效，且所有权尚未交给 TcpStream。
        unsafe { closesocket(raw) };
        e
    };

    // 源地址绑定：走 Windows 路由表，等价于按网卡出站
    let (lp, len) = sockaddr_of(&local);
    // SAFETY: lp/len 描述合法 sockaddr，内存是 Box::leak 出的静态块。
    if unsafe { connect(raw, lp, len) } != 0 {
        // SAFETY: connect 刚失败，取 WinSock 错误码。
        let e = std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() });
        return Err(close_on_err(raw, e));
    }

    // SAFETY: raw 有效，所有权转移给 TcpStream。
    Ok(unsafe { std::net::TcpStream::from_raw_socket(raw as _) })
}

/// 把 `SocketAddr` 转成 `(sockaddr 指针, 长度)`。
///
/// 与 `unix.rs` 一样 `Box::leak` 出静态块：`connect()` 只读，泄漏量可忽略。
fn sockaddr_of(addr: &SocketAddr) -> (*const SOCKADDR, i32) {
    match addr {
        SocketAddr::V4(a) => {
            let sin = SOCKADDR_IN {
                sin_family: AF_INET,
                // SAFETY: 纯字节序转换，无前置条件。
                sin_port: unsafe { htons(a.port()) },
                sin_addr: windows_sys::Win32::Networking::WinSock::IN_ADDR {
                    S_un: windows_sys::Win32::Networking::WinSock::IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(a.ip().octets()),
                    },
                },
                sin_zero: [0; 8],
            };
            (
                Box::leak(Box::new(sin)) as *const SOCKADDR_IN as *const SOCKADDR,
                std::mem::size_of::<SOCKADDR_IN>() as i32,
            )
        }
        SocketAddr::V6(a) => {
            let sin6 = SOCKADDR_IN6 {
                sin6_family: AF_INET6,
                // SAFETY: 纯字节序转换，无前置条件。
                sin6_port: unsafe { htons(a.port()) },
                sin6_flowinfo: a.flowinfo(),
                sin6_addr: IN6_ADDR {
                    u: IN6_ADDR_0 {
                        Byte: a.ip().octets(),
                    },
                },
                Anonymous: windows_sys::Win32::Networking::WinSock::SOCKADDR_IN6_0 {
                    sin6_scope_id: a.scope_id(),
                },
            };
            (
                Box::leak(Box::new(sin6)) as *const SOCKADDR_IN6 as *const SOCKADDR,
                std::mem::size_of::<SOCKADDR_IN6>() as i32,
            )
        }
    }
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

    #[test]
    fn sockaddr_len_matches_family() {
        let v4: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert_eq!(
            sockaddr_of(&v4).1,
            std::mem::size_of::<SOCKADDR_IN>() as i32
        );
        let v6: SocketAddr = "[::1]:443".parse().unwrap();
        assert_eq!(
            sockaddr_of(&v6).1,
            std::mem::size_of::<SOCKADDR_IN6>() as i32
        );
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
