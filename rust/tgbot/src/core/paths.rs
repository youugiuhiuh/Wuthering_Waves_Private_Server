//! 集中路径常量定义
//!
//! 所有系统路径在此定义，避免硬编码分散

pub const WWPS_BASE_DIR: &str = "/etc/wwps";

/// Xray-core 相关路径
pub mod xray {
    pub const DIR: &str = "/etc/wwps/wwps-core";
    pub const BIN: &str = "/etc/wwps/wwps-core/wwps-core";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-core/conf";
    pub const BACKUP_DIR: &str = "/etc/wwps/wwps-core/backup";
    pub const TEMP_DIR: &str = "/tmp/wwps-core-installer";

    pub const DEFAULT_OWNER: &str = "XTLS";
    pub const DEFAULT_REPO: &str = "Xray-core";
    pub const DEFAULT_SERVICE: &str = "wwps-core";
    pub const DEFAULT_TEMP_DIR: &str = "/tmp/wwps-core-upgrade";
    pub const DEFAULT_BACKUP_PREFIX: &str = "wwps-core-backup";
}

/// Sing-box 相关路径
pub mod singbox {
    pub const DIR: &str = "/etc/wwps/wwps-box";
    pub const BIN: &str = "/etc/wwps/wwps-box/wwps-box";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-box/conf";
    pub const CERTS_DIR: &str = "/etc/wwps/wwps-box/certs";
    pub const TLS_CERT: &str = "/etc/wwps/wwps-box/certs/tls.cer";
    pub const TLS_KEY: &str = "/etc/wwps/wwps-box/certs/tls.key";
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wwps_base_dir() {
        assert_eq!(WWPS_BASE_DIR, "/etc/wwps");
    }

    #[test]
    fn test_xray_paths() {
        assert_eq!(xray::DIR, "/etc/wwps/wwps-core");
        assert_eq!(xray::BIN, "/etc/wwps/wwps-core/wwps-core");
        assert_eq!(xray::CONF_DIR, "/etc/wwps/wwps-core/conf");
        assert_eq!(xray::DEFAULT_SERVICE, "wwps-core");
    }

    #[test]
    fn test_singbox_paths() {
        assert_eq!(singbox::DIR, "/etc/wwps/wwps-box");
        assert_eq!(singbox::BIN, "/etc/wwps/wwps-box/wwps-box");
        assert_eq!(singbox::CERTS_DIR, "/etc/wwps/wwps-box/certs");
        assert_eq!(singbox::TLS_CERT, "/etc/wwps/wwps-box/certs/tls.cer");
    }

    #[test]
    fn test_bot_paths() {
        assert_eq!(bot::DIR, "/etc/wwps/tgbot");
        assert_eq!(bot::KEY_FILE, "/etc/wwps/tgbot/.key");
    }

    #[test]
    fn test_warp_paths() {
        assert_eq!(warp::ACCOUNT_FILE, "/etc/wwps/wwps-core/warp_account.json");
        assert_eq!(warp::ROUTING_FILE, "/etc/wwps/wwps-core/conf/10_warp_routing.json");
    }

    #[test]
    fn test_log_paths() {
        assert_eq!(log::DIR, "/var/log");
    }
}