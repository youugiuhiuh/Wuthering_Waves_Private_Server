//! Google 连通性检测：判定当前网络是否已具备直连海外能力。
//!
//! 用 DNS 解析 + 完整 TLS 1.3 握手而不是单纯 TCP 443 连通性：
//! 审查系统对 Google IP 常表现为「TCP 三次握手成功、TLS ClientHello 后被 RST」，
//! 只看 TCP 会把被封锁的网络误判为可用。

use std::net::IpAddr;
use std::time::Duration;

/// 探测目标：`www.google.com` 全球任一 A 记录。
const PROBE_HOST: &str = "www.google.com";

/// 单次连通性检测结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connectivity {
    /// TLS 握手完成，可直连。
    Reachable { ip: IpAddr },
    /// DNS 解析失败（无境外 DNS 或全域不可用）。
    DnsFailed(String),
    /// 解析成功但 TLS 握手失败（典型 GFW RST）。
    HandshakeFailed { ip: IpAddr, reason: String },
}

impl Connectivity {
    pub fn is_reachable(&self) -> bool {
        matches!(self, Self::Reachable { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Reachable { ip } => format!("可连通（{PROBE_HOST} → {ip}，TLS 1.3 握手成功）"),
            Self::DnsFailed(e) => format!("不可访问：DNS 解析 {PROBE_HOST} 失败（{e}）"),
            Self::HandshakeFailed { ip, reason } => {
                format!("不可访问：{PROBE_HOST} → {ip} TLS 握手失败（{reason}）")
            }
        }
    }
}

/// 检测当前网络能否直连 Google。
pub async fn check(timeout: Duration) -> Connectivity {
    let resolver = match crate::dns::build_resolver(timeout) {
        Ok(r) => r,
        Err(e) => return Connectivity::DnsFailed(e.to_string()),
    };

    let ip = match crate::dns::resolve_first(&resolver, PROBE_HOST).await {
        Ok(ip) => ip,
        Err(e) => return Connectivity::DnsFailed(e.to_string()),
    };

    // 只做握手，不解析证书、不发 HTTP：与 probe_survival 同一套轻量路径
    match tokio::task::spawn_blocking(move || crate::tls::handshake_only(PROBE_HOST, ip, timeout))
        .await
    {
        Ok(Ok(())) => Connectivity::Reachable { ip },
        Ok(Err(e)) => Connectivity::HandshakeFailed {
            ip,
            reason: e.to_string(),
        },
        Err(e) => Connectivity::HandshakeFailed {
            ip,
            reason: format!("join error: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachable_is_the_only_success_variant() {
        let ok = Connectivity::Reachable {
            ip: "1.2.3.4".parse().unwrap(),
        };
        assert!(ok.is_reachable());
        assert!(ok.describe().contains("可连通"));

        let dns = Connectivity::DnsFailed("servfail".into());
        assert!(!dns.is_reachable());
        assert!(dns.describe().contains("DNS 解析"));

        let hs = Connectivity::HandshakeFailed {
            ip: "1.2.3.4".parse().unwrap(),
            reason: "connection reset by peer".into(),
        };
        assert!(!hs.is_reachable());
        assert!(hs.describe().contains("TLS 握手失败"));
    }

    /// 联网环境下的冒烟测试：本机若无外网会失败，因此只在显式 opt-in 时运行。
    #[tokio::test]
    async fn detects_current_network_state() {
        if std::env::var("SNI_PROBE_NET_TESTS").is_err() {
            return;
        }
        let c = check(Duration::from_secs(5)).await;
        println!("{}", c.describe());
        match c {
            Connectivity::Reachable { .. } | Connectivity::HandshakeFailed { .. } => {}
            Connectivity::DnsFailed(e) => panic!("境外 DNS 解析失败: {e}"),
        }
    }
}
