//! Linux / Unix 平台实现：`SO_BINDTODEVICE` + `getifaddrs`。
//!
//! 只在 `target_os = "linux"` 时真正启用网卡绑定；其他 Unix 由 mod.rs 挡在外面。

#![cfg(unix)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::RawFd;

/// 把 socket 绑定到指定网卡（`SO_BINDTODEVICE`）。
///
/// 需要 root 或 `CAP_NET_RAW`，普通用户会拿到 `EPERM`。
#[cfg(target_os = "linux")]
pub fn bind_socket_to_device(fd: RawFd, iface: &str) -> std::io::Result<()> {
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

/// 非 Linux 的 Unix 平台没有 `SO_BINDTODEVICE`，由 mod.rs 在入口处挡掉，
/// 这里只保留符号以让 `connect_bound` 通过编译。
#[cfg(not(target_os = "linux"))]
pub fn bind_socket_to_device(_fd: RawFd, iface: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("本平台不支持按网卡 {iface} 绑定"),
    ))
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

/// 创建一个已按当前绑定配置好的 TCP socket 并连接到目标。
///
/// 这是 Rust 侧所有 TLS 探测的统一起点：不经过 `TcpStream::connect_timeout`，
/// 因为它没有暴露 socket 选项的钩子。
pub fn connect_bound(
    target: SocketAddr,
    timeout: std::time::Duration,
    bind: &super::BindTarget,
) -> std::io::Result<std::net::TcpStream> {
    use std::os::fd::FromRawFd;

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

    // 超时：SO_SNDTIMEO 只约束 send()，在 Linux 上对 connect() 无效。
    // 必须用非阻塞 connect + poll 自己算截止时间，否则会一直等到内核
    // TCP SYN 重传超时（~130s），--timeout 形同虚设。
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

    // 设为非阻塞以便给 connect 加真超时
    // SAFETY: fd 有效；F_GETFL/F_SETFL 不改变所有权。
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(close_on_err(fd, std::io::Error::last_os_error()));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(close_on_err(fd, std::io::Error::last_os_error()));
    }

    // connect：非阻塞下立即返回 EINPROGRESS
    let (ptr, len) = sockaddr_of(&target);
    // SAFETY: ptr/len 描述合法 sockaddr，fd 有效。
    let rc = unsafe { libc::connect(fd, ptr, len) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(close_on_err(fd, err));
        }
        // 等可写 = 连接完成或失败，最多等到 timeout
        let ms = timeout.as_millis().min(u32::MAX as u128) as libc::c_int;
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: pfd 是合法的单元素数组。
        let prc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if prc == 0 {
            return Err(close_on_err(
                fd,
                std::io::Error::new(std::io::ErrorKind::TimedOut, "connect 超时"),
            ));
        }
        if prc < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                return Err(close_on_err(
                    fd,
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "connect 被中断"),
                ));
            }
            return Err(close_on_err(fd, e));
        }
        // 可写不代表成功，必须取 SO_ERROR
        let mut so_err: libc::c_int = 0;
        let mut elen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: so_err/elen 是合法的输出缓冲区。
        let grc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut so_err as *mut _ as *mut libc::c_void,
                &mut elen,
            )
        };
        if grc != 0 {
            return Err(close_on_err(fd, std::io::Error::last_os_error()));
        }
        if so_err != 0 {
            return Err(close_on_err(fd, std::io::Error::from_raw_os_error(so_err)));
        }
    }

    // 恢复阻塞模式交给 TcpStream（后续 read/write 依赖它 + SO_*TIMEO）
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
        return Err(close_on_err(fd, std::io::Error::last_os_error()));
    }

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
