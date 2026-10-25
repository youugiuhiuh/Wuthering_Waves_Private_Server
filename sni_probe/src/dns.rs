//! 纯海外 Anycast DNS 解析：严禁使用境内（CN）DNS 服务器。
//!
//! 境内解析器会记录探测方的查询行为，且对境外目标返回可能被污染的结果。

use std::net::IpAddr;
use std::time::Duration;

pub use hickory_resolver::TokioResolver;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::net::runtime::TokioRuntimeProvider;

/// 纯海外 Anycast DNS 服务器（与 Go 版 `FastDNSServers` 对齐）。
pub const FAST_DNS_SERVERS: &[&str] = &[
    "1.1.1.1",         // Cloudflare
    "1.0.0.1",         // Cloudflare
    "8.8.8.8",         // Google
    "8.8.4.4",         // Google
    "9.9.9.9",         // Quad9
    "149.112.112.112", // Quad9
    "208.67.222.222",  // OpenDNS
    "208.67.220.220",  // OpenDNS
];

/// 构建仅含海外服务器的并发解析器。
pub fn build_resolver(
    timeout: Duration,
) -> Result<TokioResolver, Box<dyn std::error::Error + Send + Sync>> {
    // 绑定了本地 IP 时，DNS 也要从同一源地址出去，否则查询会走默认网卡
    let local_ip = crate::bind::current().local_ip;

    let mut servers = Vec::with_capacity(FAST_DNS_SERVERS.len());
    for s in FAST_DNS_SERVERS {
        let ip: IpAddr = s.parse().map_err(|e| format!("bad DNS {s}: {e}"))?;
        let mut cfg = NameServerConfig::udp_and_tcp(ip);
        if let Some(local) = local_ip {
            for conn in &mut cfg.connections {
                // 源端口 0 = 由内核选择；这里只固定源 IP
                conn.bind_addr = Some(std::net::SocketAddr::new(local, 0));
            }
        }
        servers.push(cfg);
    }

    let mut opts = ResolverOpts::default();
    opts.timeout = timeout;
    opts.attempts = 2;
    // 只取 IPv4：REALITY 伪装目标以 A 记录为主，IPv6 交由后续阶段处理
    opts.ip_strategy = hickory_resolver::config::LookupIpStrategy::Ipv4Only;

    Ok(TokioResolver::builder_with_config(
        ResolverConfig::from_parts(None, vec![], servers),
        TokioRuntimeProvider::default(),
    )
    .with_options(opts)
    .build()?)
}

/// 解析域名的首个 IPv4 地址。
pub async fn resolve_first(
    resolver: &TokioResolver,
    domain: &str,
) -> Result<IpAddr, Box<dyn std::error::Error + Send + Sync>> {
    let ips = resolver.lookup_ip(domain).await?;
    ips.iter()
        .next()
        .ok_or_else(|| format!("no A record for {domain}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 境内 DNS（腾讯/Ali/360/114/百度/字节等）一律不得出现在解析器配置中。
    #[test]
    fn no_china_dns_servers() {
        const BLOCKED: &[&str] = &[
            "114.114.",
            "223.5.",
            "223.6.",
            "119.29.",
            "180.76.",
            "180.184.",
            "1.2.4.8",
            "210.2.4.8",
            "117.50.",
            "120.53.53.53",
            "1.12.12.12",
        ];
        for s in FAST_DNS_SERVERS {
            for b in BLOCKED {
                assert!(!s.starts_with(b), "境内 DNS 混入: {s}（匹配 {b}）");
            }
        }
    }

    #[test]
    fn resolver_builds_with_only_overseas_servers() {
        // 构建失败会直接 panic；服务器清单由上面 no_china_dns_servers 静态校验
        build_resolver(Duration::from_millis(500)).expect("resolver");
    }

    #[test]
    fn fast_dns_servers_are_valid_ips() {
        for s in FAST_DNS_SERVERS {
            assert!(s.parse::<IpAddr>().is_ok(), "非法 DNS 地址: {s}");
        }
    }

    #[tokio::test]
    async fn resolves_public_domain() {
        let r = build_resolver(Duration::from_secs(3)).expect("resolver");
        // example.com 的 A 记录长期稳定，作为海外解析连通性冒烟测试
        match resolve_first(&r, "example.com").await {
            Ok(ip) => assert!(ip.is_ipv4()),
            Err(e) => panic!("海外 DNS 解析失败（检查网络/代理）: {e}"),
        }
    }
}
