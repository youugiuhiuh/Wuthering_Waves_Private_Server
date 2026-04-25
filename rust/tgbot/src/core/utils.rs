//! 共享工具函数

use rand::{rngs::StdRng, Rng, SeedableRng};
use crate::core::types::{IpVersion, Result};
use crate::core::error::AppError;
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