//! 出站网卡绑定（对齐 Go `pkg/net_dialer.go` + `bind_linux.go`）。
//!
//! 用途：把测试流量锁到指定网卡或本地 IP，避免打满主宽带 / 掐断 SSH。
//!
//! **平台差异**：
//!
//! | 平台 | 按 IP 绑定 | 按网卡绑定 | 说明 |
//! | :--- | :--- | :--- | :--- |
//! | Linux | ✅ | ✅ `SO_BINDTODEVICE` | 需 root / `CAP_NET_RAW` |
//! | Windows | ✅ | ❌ **不支持** | `--iface` 只接受 IP 字面量 |
//! | 其他 Unix | ✅ | ❌ **不支持** | 同上 |
//!
//! Windows 没有 `SO_BINDTODEVICE` 的等价物（`IP_UNICAST_IF` 只影响组播、
//! 且需要按接口索引而非名字），因此按网卡绑定在该平台直接拒绝并提示改用 IP。

use std::net::{IpAddr, SocketAddr};

use crate::logging;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as plat;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as plat;

/// 按网卡名绑定是否受支持（仅在 Linux 为 true）。
pub const IFACE_BIND_SUPPORTED: bool = cfg!(target_os = "linux");

/// 已解析的出站绑定目标。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BindTarget {
    /// 需要按网卡绑定的网卡名（可能为空 = 未按网卡绑定）
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
/// 传空串表示清除绑定。非 Linux 平台给网卡名会报错并提示改用 IP。
pub fn set_outbound(iface_or_ip: &str) -> Result<(), String> {
    if iface_or_ip.is_empty() {
        *BIND.write().expect("bind lock poisoned") = BindTarget::default();
        return Ok(());
    }

    // 1. 直接给 IP（跨平台）
    if let Ok(ip) = iface_or_ip.parse::<IpAddr>() {
        *BIND.write().expect("bind lock poisoned") = BindTarget {
            iface: None,
            local_ip: Some(ip),
        };
        return Ok(());
    }

    // 2. 给网卡名：仅 Linux 支持
    if !IFACE_BIND_SUPPORTED {
        return Err(format!(
            "本平台（{}）不支持按网卡名绑定，请改用本地 IP：--iface <IP>",
            std::env::consts::OS
        ));
    }

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

/// 查询网卡上的第一个 IPv4 地址。非 Linux 返回 `None`。
pub fn iface_ipv4(name: &str) -> Option<IpAddr> {
    if !IFACE_BIND_SUPPORTED {
        return None;
    }
    plat::iface_ipv4(name)
}

/// 列出本机所有网卡名。非 Linux 返回空（该平台不支持按网卡绑定）。
pub fn list_interfaces() -> Vec<String> {
    if !IFACE_BIND_SUPPORTED {
        return Vec::new();
    }
    plat::list_interfaces()
}

/// 创建一个已按当前绑定配置好的 TCP 连接。
///
/// 这是 Rust 侧所有 TLS 探测的统一起点：不经标准库 `connect_timeout`，
/// 因为它没有暴露 socket 选项的钩子。
pub fn connect_tcp(
    target: SocketAddr,
    timeout: std::time::Duration,
) -> std::io::Result<std::net::TcpStream> {
    let bind = current();

    // 未绑定时走标准库快路径（跨平台）
    if bind.is_empty() {
        return std::net::TcpStream::connect_timeout(&target, timeout);
    }

    plat::connect_bound(target, timeout, &bind)
}

/// 绑定后的启动自检：提示当前生效的绑定信息。
pub fn validate_binding() -> Option<String> {
    let bind = current();
    if bind.is_empty() {
        return None;
    }
    if bind.iface.is_some() && bind.local_ip.is_none() {
        return Some(format!(
            "网卡 {:?} 没有可用 IPv4，绑定可能不生效",
            bind.iface
        ));
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
        assert!(!current().is_empty() || err.contains("不支持") || err.contains("不存在"));
        reset();
    }

    /// 非 Linux 平台必须明确拒绝按网卡绑定，而不是静默忽略。
    #[test]
    fn iface_binding_rejected_where_unsupported() {
        if IFACE_BIND_SUPPORTED {
            return;
        }
        reset();
        let err = set_outbound("lo").unwrap_err();
        assert!(
            err.contains("不支持按网卡名绑定"),
            "应提示改用 IP，实际: {err}"
        );
        assert!(err.contains("--iface <IP>"), "应给出具体做法: {err}");
        assert!(current().is_empty(), "被拒绝时不应改动绑定");
        reset();
    }

    #[test]
    fn connect_succeeds_unbound() {
        reset();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect_tcp(
            format!("127.0.0.1:{port}").parse().unwrap(),
            std::time::Duration::from_secs(2),
        )
        .expect("未绑定时应正常连接");
        assert_eq!(s.peer_addr().unwrap().port(), port);
    }

    #[test]
    fn connect_fails_when_nothing_listens() {
        reset();
        let r = connect_tcp(
            "127.0.0.1:1".parse().unwrap(),
            std::time::Duration::from_millis(200),
        );
        assert!(r.is_err(), "无监听端口应连接失败");
    }

    /// 按 IP 绑定后，源地址必须等于绑定的 IP —— 跨平台都应成立。
    #[test]
    fn bound_connect_uses_bound_source_ip() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();

        set_outbound("127.0.0.1").unwrap();
        let s = connect_tcp(
            format!("127.0.0.1:{port}").parse().unwrap(),
            std::time::Duration::from_secs(2),
        )
        .expect("按 IP 绑定后应能连接");
        assert_eq!(
            s.local_addr().unwrap().ip(),
            "127.0.0.1".parse::<IpAddr>().unwrap(),
            "源 IP 必须等于绑定的 IP"
        );
        reset();
    }
}
