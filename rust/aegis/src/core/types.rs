//! 共享类型定义

use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsProvider {
    Cloudflare,
    Route53,
}

impl DnsProvider {
    pub const fn acme_flag(self) -> &'static str {
        match self {
            Self::Cloudflare => "dns_cf",
            Self::Route53 => "dns_aws",
        }
    }

    pub const fn credential_names(self) -> (&'static str, &'static str) {
        match self {
            Self::Cloudflare => ("CF_Token", "CF_Zone_ID"),
            Self::Route53 => ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
        }
    }

    /// 该 provider 下应该建 inbound 的端口。
    ///
    /// Cloudflare 公开支持 443/8443/2053/2083/2087/2096 等 HTTPS 备用端口，
    /// 但在「一个域名指向源站」的场景下，CF 会在这些端口间随机选择回源端口。
    /// 若服务端在这些端口上都建了 inbound，且每个 inbound 的 `path` 不同，
    /// 客户端请求就会落到 path 不匹配的 inbound，表现为 403 或断连
    /// （实测观察到 CF 曾把请求打到 8443 与 2096）。
    ///
    /// 因此这里只返回 443，保证回源端口唯一。若将来要恢复多端口，
    /// 必须同时让所有 inbound 共用同一 `path` 与同一 UUID，否则依旧会错配。
    pub const fn cdn_ports(self) -> &'static [u16] {
        match self {
            Self::Cloudflare => &[443],
            Self::Route53 => &[443],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainFlowSource {
    Standalone,
    OneClick,
    /// 自定义放行（Xray `custom_direct`）：输入的是「要放行的域名」，不是证书申请域名。
    /// 复用同一个输入状态机，但必须在 `handle_message` 里按来源分流，绝不进入 ACME 路径。
    CustomAllowlist,
    /// 生效自检：输入仍是「要检查的域名」，但语义是【只读判定】——
    /// 单独一个变体是为了让「不写盘」在类型层面就与上面的「添加」区分开，
    /// 否则只靠函数内部的布尔开关，日后很容易把自检接回写盘路径。
    CustomAllowlistCheck,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainInputStep {
    AwaitDomain,
    AwaitProvider,
    AwaitCredentials(DnsProvider),
    Processing,
}

#[derive(Debug, Clone)]
pub struct DomainInputState {
    pub updated_at: Instant,
    pub source: DomainFlowSource,
    pub step: DomainInputStep,
    pub domain: Option<String>,
}

/// 批量创建结果
#[derive(Debug, Clone, Default)]
pub struct BatchCreationResult {
    pub links: Vec<String>,
    pub config_file: Option<String>,
    pub backup_file: Option<String>,
    pub created_count: usize,
}

impl BatchCreationResult {
    pub fn new(links: Vec<String>, config_file: Option<String>, created_count: usize) -> Self {
        Self {
            links,
            config_file,
            backup_file: None,
            created_count,
        }
    }
}

/// IP 版本选择
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IpVersion {
    #[default]
    IPv4,
    IPv6,
    SplitStackV6Primary,
    SplitStackV4Primary,
}

impl IpVersion {
    pub fn is_ipv6_primary(&self) -> bool {
        matches!(self, IpVersion::IPv6 | IpVersion::SplitStackV6Primary)
    }

    pub fn is_ipv4_primary(&self) -> bool {
        matches!(self, IpVersion::IPv4 | IpVersion::SplitStackV4Primary)
    }

    pub fn label(&self) -> &'static str {
        match self {
            IpVersion::IPv4 => "IPv4",
            IpVersion::IPv6 => "IPv6",
            IpVersion::SplitStackV6Primary => "IPv6/IPv4",
            IpVersion::SplitStackV4Primary => "IPv4/IPv6",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_provider_maps_to_acme_contract() {
        assert_eq!(DnsProvider::Cloudflare.acme_flag(), "dns_cf");
        assert_eq!(
            DnsProvider::Cloudflare.credential_names(),
            ("CF_Token", "CF_Zone_ID")
        );
        assert_eq!(DnsProvider::Route53.acme_flag(), "dns_aws");
        assert_eq!(
            DnsProvider::Route53.credential_names(),
            ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY")
        );
    }

    /// Cloudflare 只保留 443。
    ///
    /// 同一域名下 CF 会随机回源到它支持的多个 HTTPS 备用端口之一；
    /// 若服务端在这些端口都建了 inbound 且 path 各不相同，回源会落到错误的
    /// inbound，表现为 403 或断连。只保留 443 使回源端口唯一。
    #[test]
    fn dns_provider_cdn_ports() {
        assert_eq!(DnsProvider::Cloudflare.cdn_ports(), &[443]);
        assert_eq!(DnsProvider::Route53.cdn_ports(), &[443]);
    }

    /// 所有 CDN provider 都必须只产出 443，避免任何一家回源端口歧义。
    #[test]
    fn all_providers_expose_only_port_443() {
        for p in [DnsProvider::Cloudflare, DnsProvider::Route53] {
            assert_eq!(
                p.cdn_ports(),
                &[443],
                "{:?} 只应产出 443，否则 CF 回源会随机落到其它端口",
                p
            );
        }
    }

    #[test]
    fn test_batch_creation_result_new() {
        let result = BatchCreationResult::new(
            vec!["link1".to_string(), "link2".to_string()],
            Some("config.json".to_string()),
            2,
        );
        assert_eq!(result.links.len(), 2);
        assert_eq!(result.config_file, Some("config.json".to_string()));
        assert_eq!(result.backup_file, None);
        assert_eq!(result.created_count, 2);
    }

    #[test]
    fn test_batch_creation_result_default() {
        let result = BatchCreationResult::default();
        assert!(result.links.is_empty());
        assert_eq!(result.config_file, None);
        assert_eq!(result.backup_file, None);
        assert_eq!(result.created_count, 0);
    }

    #[test]
    fn test_ip_version_default() {
        let ip: IpVersion = IpVersion::default();
        assert_eq!(ip, IpVersion::IPv4);
    }

    #[test]
    fn test_ip_version_is_ipv6_primary() {
        assert!(!IpVersion::IPv4.is_ipv6_primary());
        assert!(IpVersion::IPv6.is_ipv6_primary());
        assert!(IpVersion::SplitStackV6Primary.is_ipv6_primary());
        assert!(!IpVersion::SplitStackV4Primary.is_ipv6_primary());
    }

    #[test]
    fn test_ip_version_is_ipv4_primary() {
        assert!(IpVersion::IPv4.is_ipv4_primary());
        assert!(!IpVersion::IPv6.is_ipv4_primary());
        assert!(!IpVersion::SplitStackV6Primary.is_ipv4_primary());
        assert!(IpVersion::SplitStackV4Primary.is_ipv4_primary());
    }

    #[test]
    fn test_ip_version_label() {
        assert_eq!(IpVersion::IPv4.label(), "IPv4");
        assert_eq!(IpVersion::IPv6.label(), "IPv6");
        assert_eq!(IpVersion::SplitStackV6Primary.label(), "IPv6/IPv4");
        assert_eq!(IpVersion::SplitStackV4Primary.label(), "IPv4/IPv6");
    }

    #[test]
    fn test_ip_version_serialization() {
        let json = serde_json::to_string(&IpVersion::IPv6).unwrap();
        assert_eq!(json, "\"IPv6\"");
    }

    #[test]
    fn test_ip_version_deserialization() {
        let ip: IpVersion = serde_json::from_str("\"SplitStackV4Primary\"").unwrap();
        assert_eq!(ip, IpVersion::SplitStackV4Primary);
    }
}
