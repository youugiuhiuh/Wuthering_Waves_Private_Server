//! 集中路径常量定义
//!
//! 所有系统路径在此定义，避免硬编码分散

use std::path::PathBuf;

pub const WWPS_BASE_DIR: &str = "/etc/wwps";

/// Xray-core 相关路径
pub mod xray {
    pub const DIR: &str = "/etc/wwps/wwps-core";
    pub const BIN: &str = "/etc/wwps/wwps-core/wwps-core";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-core/conf";
    pub const ACCESS_LOG: &str = "/etc/wwps/wwps-core/access.log";
    pub const ERROR_LOG: &str = "/etc/wwps/wwps-core/error.log";
    pub const BACKUP_DIR: &str = "/etc/wwps/wwps-core/backup";
    pub const TEMP_DIR: &str = "/tmp/wwps-core-installer";

    pub fn conf_file(name: &str) -> PathBuf {
        PathBuf::from(format!("{}/{}", CONF_DIR, name))
    }
}

/// Sing-box 相关路径
pub mod singbox {
    pub const DIR: &str = "/etc/wwps/wwps-box";
    pub const BIN: &str = "/etc/wwps/wwps-box/sing-box";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-box/conf";
    pub const CERTS_DIR: &str = "/etc/wwps/wwps-box/certs";
    pub const TLS_CERT: &str = "/etc/wwps/wwps-box/certs/tls.cer";
    pub const TLS_KEY: &str = "/etc/wwps/wwps-box/certs/tls.key";

    pub fn conf_file(name: &str) -> PathBuf {
        PathBuf::from(format!("{}/{}", CONF_DIR, name))
    }
}

/// Bot 相关路径
pub mod bot {
    pub const DIR: &str = "/etc/wwps/tgbot";
    pub const KEY_FILE: &str = "/etc/wwps/tgbot/.key";
    pub const BBR3_PENDING_FLAG: &str = "/etc/wwps/tgbot/bbr3_pending.flag";
}

/// WARP 相关路径
pub mod warp {
    pub const ACCOUNT_FILE: &str = "/etc/wwps/wwps-core/warp_account.json";
    pub const ROUTING_FILE: &str = "/etc/wwps/wwps-core/conf/10_warp_routing.json";
}

/// 日志目录
pub mod log {
    pub const DIR: &str = "/var/log";
    pub const SINGBOX_LOG: &str = "/var/log/sing-box.log";
}