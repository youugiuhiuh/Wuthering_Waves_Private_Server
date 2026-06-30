//! 共享类型定义

use serde::{Deserialize, Serialize};

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

/// Internet Protocol version selection.
///
/// # Examples
///
/// ```
/// use aegis::core::types::IpVersion;
/// let v4 = IpVersion::IPv4;
/// assert_eq!(v4.label(), "IPv4");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
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

    /// Returns the human-readable label for this [`IpVersion`].
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
