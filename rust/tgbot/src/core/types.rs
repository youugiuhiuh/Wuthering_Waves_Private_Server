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