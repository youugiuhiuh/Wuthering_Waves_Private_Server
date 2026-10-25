//! Cloudflare 出口 IP 排除：REALITY 与 Cloudflare 边缘代理根本不兼容（客户端 40ms 内被断开），
//! 因此在任何阶段都必须提前剔除。
//!
//! 覆盖 AS13335 / AS209242 公开网段。

use std::net::IpAddr;
use std::sync::OnceLock;

/// Cloudflare 公开 IPv4/IPv6 网段（与 Go 版 `sni_tester/pkg/cloudflare.go` 保持一致）。
const CLOUDFLARE_CIDRS: &[&str] = &[
    // IPv4 (AS13335)
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "1.1.1.0/24",
    "1.0.0.0/24",
    // AS209242 (Cloudflare London)
    "199.27.128.0/21",
    // IPv6
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

fn nets() -> &'static [ipnet::IpNet] {
    static NETS: OnceLock<Vec<ipnet::IpNet>> = OnceLock::new();
    NETS.get_or_init(|| {
        CLOUDFLARE_CIDRS
            .iter()
            .map(|c| c.parse().expect("静态 CIDR 必须合法"))
            .collect()
    })
}

/// 判断 IP 是否落在 Cloudflare 网段内。
pub fn is_cloudflare_ip(ip: IpAddr) -> bool {
    nets().iter().any(|n| n.contains(&ip))
}

/// 综合判定：ASN/Org 归属 Cloudflare 时同样视为不可用目标。
///
/// Go 版 `IsCloudflare` 用 ASN 13335/209242 或 Org 含 `cloudflare` 兜底，
/// 这里保留同等判定，用于持有 IPinfo ASN 结果的调用方。
#[allow(dead_code)] // 供持有 IPinfo ASN 的调用方使用
pub fn is_cloudflare(asn: Option<(u32, &str)>) -> bool {
    match asn {
        Some((number, org)) => {
            number == 13335 || number == 209242 || org.to_lowercase().contains("cloudflare")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cf(s: &str) -> bool {
        is_cloudflare_ip(s.parse().unwrap())
    }

    #[test]
    fn detects_cloudflare_edges() {
        assert!(cf("1.1.1.1"), "Cloudflare DNS");
        assert!(cf("104.16.132.229"), "Cloudflare CDN");
        assert!(cf("172.64.80.1"), "Cloudflare CDN");
        assert!(cf("2606:4700:4700::1111"), "Cloudflare IPv6");
    }

    #[test]
    fn allows_non_cloudflare() {
        assert!(!cf("8.8.8.8"), "Google DNS");
        assert!(!cf("93.184.216.34"), "example.com");
        assert!(!cf("23.45.34.110"), "Akamai");
        assert!(!cf("144.76.0.1"), "Hetzner");
        assert!(!cf("2001:4860:4860::8888"), "Google IPv6");
    }

    #[test]
    fn detects_cloudflare_by_asn_or_org() {
        assert!(is_cloudflare(Some((13335, "Cloudflare, Inc."))));
        assert!(is_cloudflare(Some((209242, "Cloudflare London, LLC"))));
        assert!(is_cloudflare(Some((64500, "CLOUDFLARE"))));
        assert!(!is_cloudflare(Some((15169, "Google LLC"))));
        assert!(!is_cloudflare(None));
    }

    #[test]
    fn cidr_table_is_valid_and_covers_both_families() {
        assert!(nets().iter().any(|n| n.addr().is_ipv4()));
        assert!(nets().iter().any(|n| n.addr().is_ipv6()));
    }
}
