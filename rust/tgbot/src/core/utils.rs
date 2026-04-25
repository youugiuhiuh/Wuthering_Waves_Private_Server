//! 共享工具函数

use rand::{rngs::StdRng, Rng, SeedableRng};
use crate::core::types::IpVersion;
use crate::core::error::{AppError, Result};
use crate::logic::maintenance::MaintenanceManager;

/// 通用端口选择
pub async fn select_available_port(preferred: Option<u16>) -> Result<u16> {
    if let Some(port) = preferred {
        if MaintenanceManager::is_port_available(port).await {
            return Ok(port);
        }
    }
    
    let mut rng = StdRng::from_entropy();
    for _ in 0..1000 {
        let port = rng.gen_range(10000..60000);
        if MaintenanceManager::is_port_available(port).await {
            return Ok(port);
        }
    }
    
    Err(AppError::PortUnavailable(0))
}

/// 生成随机字符串后缀
pub fn generate_random_suffix(length: usize) -> String {
    let mut rng = StdRng::from_entropy();
    let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..chars.len());
            chars[idx] as char
        })
        .collect()
}

/// 生成时间戳+随机后缀的文件名
pub fn generate_timestamp_filename(prefix: &str, extension: &str) -> String {
    let timestamp = chrono::Utc::now().timestamp();
    let suffix = generate_random_suffix(8);
    format!("{}_{}_{}.{}", prefix, timestamp, suffix, extension)
}

/// 安全地解析 IP 版本字符串
pub fn parse_ip_version(s: &str) -> Option<IpVersion> {
    match s {
        "4" | "ipv4" | "IPv4" => Some(IpVersion::IPv4),
        "6" | "ipv6" | "IPv6" => Some(IpVersion::IPv6),
        "split6" => Some(IpVersion::SplitStackV6Primary),
        "split4" => Some(IpVersion::SplitStackV4Primary),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_suffix_length() {
        assert_eq!(generate_random_suffix(0).len(), 0);
        assert_eq!(generate_random_suffix(5).len(), 5);
        assert_eq!(generate_random_suffix(16).len(), 16);
    }

    #[test]
    fn test_generate_random_suffix_characters() {
        let suffix = generate_random_suffix(1000);
        assert!(suffix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn test_generate_random_suffix_deterministic() {
        let s1 = generate_random_suffix(8);
        let s2 = generate_random_suffix(8);
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_generate_timestamp_filename_format() {
        let filename = generate_timestamp_filename("config", "json");
        let parts: Vec<&str> = filename.split('_').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "config");
        assert!(parts[1].parse::<i64>().is_ok());
        assert!(parts[2].ends_with(".json"));
    }

    #[test]
    fn test_generate_timestamp_filename_extension_without_dot() {
        let filename = generate_timestamp_filename("test", "log");
        assert!(filename.ends_with(".log"));
    }

    #[test]
    fn test_parse_ip_version_ipv4_variants() {
        assert_eq!(parse_ip_version("4"), Some(IpVersion::IPv4));
        assert_eq!(parse_ip_version("ipv4"), Some(IpVersion::IPv4));
        assert_eq!(parse_ip_version("IPv4"), Some(IpVersion::IPv4));
    }

    #[test]
    fn test_parse_ip_version_ipv6_variants() {
        assert_eq!(parse_ip_version("6"), Some(IpVersion::IPv6));
        assert_eq!(parse_ip_version("ipv6"), Some(IpVersion::IPv6));
        assert_eq!(parse_ip_version("IPv6"), Some(IpVersion::IPv6));
    }

    #[test]
    fn test_parse_ip_version_split_stack() {
        assert_eq!(parse_ip_version("split6"), Some(IpVersion::SplitStackV6Primary));
        assert_eq!(parse_ip_version("split4"), Some(IpVersion::SplitStackV4Primary));
    }

    #[test]
    fn test_parse_ip_version_invalid() {
        assert_eq!(parse_ip_version(""), None);
        assert_eq!(parse_ip_version("invalid"), None);
        assert_eq!(parse_ip_version("ipv7"), None);
        assert_eq!(parse_ip_version("v4"), None);
        assert_eq!(parse_ip_version("v6"), None);
    }
}