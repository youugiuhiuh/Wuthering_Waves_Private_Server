// 功能域子模块
pub mod network;
pub mod security;
pub mod singbox; // 已有，不动
pub mod sni;
pub mod system;
pub mod xraycore;

// 根级模块（不动）
pub mod bot_upgrade;
pub mod cmd_async;
pub mod core_upgrade;
pub mod totp;
pub mod utils;

pub use bot_upgrade::{ReleaseArtifact, UPGRADE_FLAG_FILE, UpgradeManager};
pub use core_upgrade::{
    CpuArch, WwpsCoreReleaseInfo, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager,
};

// ========== 向后兼容 re-export ==========
// 原 logic::security (SecurityManager) → logic::security::crypto
pub use security::SecurityManager;

// 原 logic::anti_debug → logic::security::anti_debug
pub use security::anti_debug;

// 原 logic::fail2ban → logic::security::fail2ban
pub use security::fail2ban;

// 原 logic::firewall → logic::security::firewall
pub use security::firewall;

// 原 logic::firewall_scanner → logic::security::firewall_scanner
pub use security::firewall_scanner;

// 原 logic::firewalld → logic::security::firewalld
pub use security::firewalld;

// 原 logic::self_destruct → logic::security::self_destruct
pub use security::self_destruct;

// 原 logic::tls_probe → logic::security::tls_probe
pub use security::tls_probe;

// 原 logic::ufw → logic::security::ufw
pub use security::ufw;

// 原 logic::config → logic::xraycore::config
pub use xraycore::config;

// 原 logic::installer → logic::xraycore::installer
pub use xraycore::installer;

// 原 logic::port_allocator → logic::xraycore::port_allocator
pub use xraycore::port_allocator;

// 原 logic::sni_selector → logic::sni::selector
pub use sni::selector as sni_selector;

// 原 logic::sni_state → logic::sni::state
pub use sni::state as sni_state;

// 原 logic::geoip → logic::network::geoip
pub use network::geoip;

// 原 logic::warp_api → logic::network::warp_api
pub use network::warp_api;

// 原 bot_upgrade/core_upgrade 共享的 Release API 类型和类型别名
pub use network::release_api::{
    ReleaseAsset as LogicReleaseAsset, ReleaseResponse as LogicReleaseResponse, SHA256_LINE_RE,
    extract_sha256_from_body, fetch_json_from_mirrors, parse_digest, parse_sha256_manifest,
};

// 原 logic::log_audit → logic::system::log_audit
pub use system::log_audit;

// 原 logic::maintenance → logic::system::maintenance
pub use system::maintenance;

// 原 logic::system → logic::system::monitor
pub use system::monitor as system_monitor;

// 原 logic::operations → logic::system::operations
pub use system::operations;

// 原 logic::scheduler → logic::system::scheduler
pub use system::scheduler;
