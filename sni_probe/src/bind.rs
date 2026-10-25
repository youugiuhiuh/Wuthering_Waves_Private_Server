//! 出站网卡绑定（对齐 Go `pkg/net_dialer.go` + `bind_linux.go`）。
//!
//! 用途：把测试流量锁到指定网卡或本地 IP，避免打满主宽带 / 掐断 SSH。
//!
//! 两种绑法，能力不同：
//!
//! | 方式 | 机制 | UDP(DNS) | TCP(TLS) | 需要权限 |
//! | :--- | :--- | :--- | :--- | :--- |
//! | 按 IP | `bind()` 到本地地址 | ✅ | ✅ | 否 |
//! | 按网卡 | `SO_BINDTODEVICE` | ✅ | ✅ | **root / CAP_NET_RAW** |
//!
//! 给定网卡名时两种都用（先取该网卡的 IPv4 作为 `bind()` 地址，
//! 再叠 `SO_BINDTODEVICE` 保证走同一块网卡），与 Go 版行为一致。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::logging;

/// 已解析的出站绑定目标。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BindTarget {
    /// 需要 `SO_BINDTODEVICE` 的网卡名（可能为空 = 未按网卡绑定）
    pub iface: Option<String>,
    /// 本地源 IP（可能是从网卡上取到的 IPv4）
    pub local_ip: Option<IpAddr>,
}

impl BindTarget {
    pub fn is_empty(&self) -> bool {
        self.iface.is_none() && self.local_ip.is_none()
    }
}

/// 全局出站绑定（进程内唯一，等价 Go 的 `boundInterface` / `boundIP`）。
static BIND: std::sync::RwLock<BindTarget> = std::sync::RwLock::new(BindTarget {
    iface: None,
    local_ip: None,
});

/// 设置出站绑定。入参可以是网卡名（`wlp6s0`）或 IP 字面量。
///
/// 传空串表示清除绑定。
pub fn set_outbound(iface_or_ip: &str) -> Result<(), String> {
    if iface_or_ip.is_empty() {
        *BIND.write().expect("bind lock poisoned") = BindTarget::default();
        return Ok(());
    }

    // 1. 直接给 IP
    if let Ok(ip) = iface_or_ip.parse::<IpAddr>() {
        *BIND.write().expect("bind lock poisoned") = BindTarget {
            iface: None,
            local_ip: Some(ip),
        };
        return Ok(());
    }

    // 2. 给网卡名：取它的 IPv4
    let ip = iface_ipv4(iface_or_ip)
        .ok_or_else(|| format!("网卡 {iface_or_ip} 不存在，或没有可用的 IPv4 地址"))?;

    *BIND.write().expect("bind lock poisoned") = BindTarget {
        iface: Some(iface_or_ip.to_string()),
        local_ip: Some(ip),
    };
    Ok(())
}

/// 当前绑定。
pub fn current() -> BindTarget {
    BIND.read().expect("bind lock poisoned").clone()
}

/// 查询网卡上的第一个 IPv4 地址。
pub fn iface_ipv4(name: &str) -> Option<IpAddr> {
    use std::ffi::CStr;

    // SAFETY: if_nametoindex 只读传入的 C 字符串；失败返回 0。
    let cname = std::ffi::CString::new(name).ok()?;
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if idx == 0 {
        return None;
    }

    // getifaddrs 遍历地址
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs 分配链表并写回指针；失败返回 -1。
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return None;
    }

    let mut result = None;
    let mut cur = ifap;
    while !cur.is_null() {
        // SAFETY: cur 来自 getifaddrs 分配的链表，非空且有效。
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;

        if entry.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: ifa_addr 非空且指向有效的 sockaddr。
        let family = unsafe { (*entry.ifa_addr).sa_family } as i32;
        if family != libc::AF_INET {
            continue;
        }
        // SAFETY: ifa_name 由系统填充，是合法 C 字符串。
        let ifname = unsafe { CStr::from_ptr(entry.ifa_name) };
        if ifname.to_str() != Ok(name) {
            continue;
        }
        // SAFETY: family 已确认 AF_INET，可按 sockaddr_in 解释。
        let sin = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
        let octets = sin.sin_addr.s_addr.to_ne_bytes();
        result = Some(IpAddr::V4(Ipv4Addr::from(octets)));
        break;
    }

    // SAFETY: ifap 是 getifaddrs 返回的链表头，必须 freeifaddrs 释放。
    unsafe { libc::freeifaddrs(ifap) };
    result
}

/// 列出本机所有网卡名（供 `--list-ifaces` 与错误提示使用）。
pub fn list_interfaces() -> Vec<String> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: 同上，getifaddrs 失败返回非 0。
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return out;
    }
    let mut cur = ifap;
    while !cur.is_null() {
        // SAFETY: 链表节点有效。
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;
        // SAFETY: ifa_name 是合法 C 字符串。
        let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .to_string();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    // SAFETY: 释放链表。
    unsafe { libc::freeifaddrs(ifap) };
    out.sort();
    out
}

/// 把 socket 绑定到指定网卡（`SO_BINDTODEVICE`）。
///
/// 需要 root 或 `CAP_NET_RAW`，普通用户会拿到 `EPERM`。
pub fn bind_socket_to_device(fd: std::os::fd::RawFd, iface: &str) -> std::io::Result<()> {
    let name = std::ffi::CString::new(iface)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "网卡名含 NUL"))?;

    const SO_BINDTODEVICE: libc::c_int = 25; // 与 Linux 头文件一致
    // SAFETY: fd 由调用方保证有效；optval 指向长度为 len 的 C 字符串。
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_BINDTODEVICE,
            name.as_ptr() as *const libc::c_void,
            name.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// 创建一个已按当前绑定配置好的 TCP socket 并连接到目标。
///
/// 这是 Rust 侧所有 TLS 探测的统一起点：不经过 `TcpStream::connect_timeout`，
/// 因为它没有暴露 socket 选项的钩子。
pub fn connect_tcp(
    target: SocketAddr,
    timeout: std::time::Duration,
) -> std::io::Result<std::net::TcpStream> {
    use std::os::fd::FromRawFd;

    let bind = current();

    // 未绑定时走标准库快路径
    if bind.is_empty() {
        return std::net::TcpStream::connect_timeout(&target, timeout);
    }

    let domain = match target {
        SocketAddr::V4(_) => libc::AF_INET,
        SocketAddr::V6(_) => libc::AF_INET6,
    };
    // SAFETY: socket() 返回 fd 或 -1。
    let fd = unsafe { libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // 从这里开始 fd 归我们管，任何失败都要 close
    let close_on_err = |fd: libc::c_int, e: std::io::Error| -> std::io::Error {
        // SAFETY: fd 有效且未被移交给 TcpStream。
        unsafe { libc::close(fd) };
        e
    };

    // SO_BINDTODEVICE 必须在 connect 前设置
    if let Some(iface) = bind.iface.as_deref()
        && let Err(e) = bind_socket_to_device(fd, iface)
    {
        return Err(close_on_err(
            fd,
            std::io::Error::new(
                e.kind(),
                format!("绑定网卡 {iface} 失败（需要 root 或 CAP_NET_RAW）: {e}"),
            ),
        ));
    }

    // bind() 到网卡 IP，保证源地址走这块网卡
    if let Some(ip) = bind.local_ip {
        let local = SocketAddr::new(ip, 0);
        let (ptr, len) = sockaddr_of(&local);
        // SAFETY: ptr/len 描述一个合法的 sockaddr。
        let rc = unsafe { libc::bind(fd, ptr, len) };
        if rc != 0 {
            return Err(close_on_err(fd, std::io::Error::last_os_error()));
        }
    }

    // 超时
    let tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: timeout.subsec_micros() as libc::suseconds_t,
    };
    // SAFETY: fd 有效，tv 是合法 timeval。
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }

    // connect
    let (ptr, len) = sockaddr_of(&target);
    // SAFETY: ptr/len 描述合法 sockaddr，fd 有效。
    let rc = unsafe { libc::connect(fd, ptr, len) };
    if rc != 0 {
        return Err(close_on_err(fd, std::io::Error::last_os_error()));
    }

    // 恢复为阻塞 socket 交给 TcpStream
    // SAFETY: fd 有效且所有权转移给 TcpStream。
    Ok(unsafe { std::net::TcpStream::from_raw_fd(fd) })
}

/// 把 `SocketAddr` 转成 `(sockaddr 指针, 长度)`。
fn sockaddr_of(addr: &SocketAddr) -> (*const libc::sockaddr, libc::socklen_t) {
    match addr {
        SocketAddr::V4(a) => {
            let sin = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: a.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(a.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            // 泄漏到栈上引用：调用方立即使用，不跨语句
            (
                Box::leak(Box::new(sin)) as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        }
        SocketAddr::V6(a) => {
            let sin6 = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: a.port().to_be(),
                sin6_flowinfo: a.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: a.ip().octets(),
                },
                sin6_scope_id: a.scope_id(),
            };
            (
                Box::leak(Box::new(sin6)) as *const libc::sockaddr_in6 as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        }
    }
}

/// 绑定后的启动自检：网卡存在且有 IPv4，权限够不够。
///
/// 返回警告文本；一切正常返回 `None`。
pub fn validate_binding() -> Option<String> {
    let bind = current();
    if bind.is_empty() {
        return None;
    }
    if let Some(iface) = bind.iface.as_deref()
        && bind.local_ip.is_none()
    {
        return Some(format!("网卡 {iface} 没有可用 IPv4，绑定可能不生效"));
    }
    logging::is_debug()
        .then(|| format!("出站绑定：网卡={:?} 本地IP={:?}", bind.iface, bind.local_ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        set_outbound("").unwrap();
    }

    #[test]
    fn empty_clears_binding() {
        reset();
        assert!(current().is_empty());
    }

    #[test]
    fn ip_literal_binds_without_iface() {
        reset();
        set_outbound("127.0.0.1").unwrap();
        let b = current();
        assert_eq!(b.local_ip, Some("127.0.0.1".parse().unwrap()));
        assert!(b.iface.is_none(), "给 IP 时不该去猜网卡");
        reset();
    }

    #[test]
    fn ipv6_literal_also_accepted() {
        reset();
        set_outbound("::1").unwrap();
        assert!(current().local_ip.is_some());
        reset();
    }

    #[test]
    fn unknown_interface_is_an_error() {
        reset();
        let err = set_outbound("definitely_not_a_real_iface0").unwrap_err();
        assert!(err.contains("不存在"), "错误信息应说明原因: {err}");
        assert!(current().is_empty(), "失败不应改动现有绑定");
    }

    #[test]
    fn loopback_interface_resolves_to_ipv4() {
        let ip = iface_ipv4("lo");
        assert_eq!(
            ip,
            Some("127.0.0.1".parse().unwrap()),
            "lo 必须是 127.0.0.1"
        );
    }

    #[test]
    fn interface_name_binds_both_iface_and_ip() {
        reset();
        if iface_ipv4("lo").is_none() {
            return; // 环境无 lo，跳过
        }
        set_outbound("lo").unwrap();
        let b = current();
        assert_eq!(b.iface.as_deref(), Some("lo"));
        assert!(b.local_ip.is_some(), "按网卡绑定时应同时拿到本地 IP");
        reset();
    }

    #[test]
    fn list_interfaces_contains_loopback() {
        let list = list_interfaces();
        assert!(!list.is_empty(), "至少应有 lo");
        assert!(list.contains(&"lo".to_string()), "应包含 lo: {list:?}");
    }

    /// 明确记录：非 root 用户下 `SO_BINDTODEVICE` 应返回 EPERM。
    /// 这条测试不假设运行身份，两种结果都必须给出可诊断的信息。
    #[test]
    fn device_bind_reports_permission_or_succeeds() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        set_outbound("lo").unwrap();
        let r = connect_tcp(
            format!("127.0.0.1:{port}").parse().unwrap(),
            std::time::Duration::from_secs(2),
        );
        match r {
            Ok(_) => eprintln!("以 root/CAP_NET_RAW 运行：SO_BINDTODEVICE 生效"),
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("CAP_NET_RAW"),
                    "非 root 时应提示需要 CAP_NET_RAW，实际错误: {msg}"
                );
                eprintln!("非 root：{msg}");
            }
        }
        reset();
    }

    #[test]
    fn unbound_connect_works_normally() {
        reset();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect_tcp(
            format!("127.0.0.1:{port}").parse().unwrap(),
            std::time::Duration::from_secs(2),
        )
        .expect("未绑定时应正常连接");
        assert!(s.peer_addr().unwrap().port() == port);
    }

    /// 绑定网卡后，连接应真的从该网卡的源地址发出。
    /// 用 `TcpStream::local_addr()` 验证源 IP —— 这是绑定生效的硬证据。
    #[test]
    fn bound_connection_source_ip_matches_interface() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();

        set_outbound("lo").unwrap();
        let want = iface_ipv4("lo").expect("lo 应有 IPv4");

        match connect_tcp(
            format!("127.0.0.1:{port}").parse().unwrap(),
            std::time::Duration::from_secs(2),
        ) {
            Ok(s) => {
                let src = s.local_addr().expect("应有本地地址").ip();
                assert_eq!(src, want, "源 IP 必须等于绑定网卡的 IP");
                eprintln!("绑定生效：源 IP = {src}");
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("CAP_NET_RAW"),
                    "失败时必须提示权限问题，实际: {msg}"
                );
            }
        }
        reset();
    }

    #[test]
    fn connect_refuses_when_nothing_listens() {
        reset();
        // 9999 端口大概率没有服务；用 connect 必然失败验证错误路径不 panic
        let r = connect_tcp(
            "127.0.0.1:1".parse().unwrap(),
            std::time::Duration::from_millis(200),
        );
        assert!(r.is_err(), "无监听端口应连接失败");
    }
}
