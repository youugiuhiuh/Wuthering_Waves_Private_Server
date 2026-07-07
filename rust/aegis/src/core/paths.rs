//! 集中路径常量定义
//!
//! 所有系统路径在此定义，避免硬编码分散

pub const WWPS_BASE_DIR: &str = "/etc/wwps";

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
    pub const DIR: &str = "/etc/wwps/aegis";
    pub const KEY_FILE: &str = "/etc/wwps/aegis/.key";
    pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/aegis/bbr3_pending.flag";
}

/// Xray-core 相关路径
pub mod xray {
    pub const DIR: &str = "/etc/wwps/wwps-core";
    pub const BIN: &str = "/etc/wwps/wwps-core/wwps-core";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-core/conf";
    pub const BACKUP_DIR: &str = "/etc/wwps/wwps-core/backup";
    pub const TEMP_DIR: &str = "/tmp/wwps-core-installer";
    pub const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
    pub const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";

    pub const DEFAULT_OWNER: &str = "XTLS";
    pub const DEFAULT_REPO: &str = "Xray-core";
    pub const DEFAULT_SERVICE: &str = "wwps-core";
    pub const DEFAULT_TEMP_DIR: &str = "/tmp/wwps-core-upgrade";
    pub const DEFAULT_BACKUP_PREFIX: &str = "wwps-core-backup";
}

/// Maintenance 相关路径
pub mod maintenance {
    pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/aegis/bbr3_pending.flag";
    pub const UPGRADE_FLAG_FILE: &str = "/etc/wwps/aegis/upgrade.flag";
    pub const UNATTENDED_UPGRADES_CONF: &str = "/etc/apt/apt.conf.d/50unattended-upgrades";
    pub const AUTO_UPGRADES_PERIODIC_CONF: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
    pub const DNF_AUTOMATIC_CONF: &str = "/etc/dnf/automatic.conf";
    pub const NEEDRESTART_CONF: &str = "/etc/needrestart/needrestart.conf";
    pub const REBOOT_REQUIRED_FLAG: &str = "/var/run/reboot-required";
    pub const DESTRUCT_TARGETS: &[&str] = &[
        "/etc/wwps",
        "/var/log",
        "/root/.acme.sh",
        "/etc/systemd/system/wwps-aegis.service",
    ];
    pub const DESTRUCT_SERVICES: &[&str] = &["wwps-core", "wwps-box", "nginx"];
}

/// WARP 相关路径
pub mod warp {
    pub const ACCOUNT_FILE: &str = "/etc/wwps/wwps-core/warp_account.json";
    pub const ROUTING_FILE: &str = "/etc/wwps/wwps-core/conf/10_warp_routing.json";
}

/// Subscription server paths
pub mod sub_server {
    pub const BIN: &str = "/usr/local/bin/sub-server";
    pub const DIR: &str = "/etc/wwps/sub-server";
    pub const CERTS_DIR: &str = "/etc/wwps/sub-server/certs";
    pub const TLS_CERT: &str = "/etc/wwps/sub-server/certs/fullchain.pem";
    pub const TLS_KEY: &str = "/etc/wwps/sub-server/certs/privkey.pem";
    pub const GRPC_SOCK: &str = "/var/run/aegis/sub.sock";
    pub const SERVICE: &str = "wwps-sub-server";
    pub const CONFIG_FILE: &str = "/etc/wwps/sub-server/config.json";
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
        assert_eq!(bot::DIR, "/etc/wwps/aegis");
        assert_eq!(bot::KEY_FILE, "/etc/wwps/aegis/.key");
        assert_eq!(
            bot::BBR3_PENDING_FLAG_FILE,
            "/etc/wwps/aegis/bbr3_pending.flag"
        );
    }

    #[test]
    fn test_xray_pq_paths() {
        assert_eq!(xray::PQ_SEED_PATH, "/etc/wwps/reality_pq.seed");
        assert_eq!(xray::PQ_PUB_PATH, "/etc/wwps/reality_pq.pub");
    }

    #[test]
    fn test_maintenance_paths() {
        assert_eq!(
            maintenance::BBR3_PENDING_FLAG_FILE,
            "/etc/wwps/aegis/bbr3_pending.flag"
        );
        assert_eq!(
            maintenance::UPGRADE_FLAG_FILE,
            "/etc/wwps/aegis/upgrade.flag"
        );
        assert!(!maintenance::DESTRUCT_TARGETS.is_empty());
        assert!(maintenance::DESTRUCT_TARGETS.contains(&"/etc/wwps"));
        assert!(!maintenance::DESTRUCT_SERVICES.is_empty());
        assert!(maintenance::DESTRUCT_SERVICES.contains(&"wwps-core"));
    }

    #[test]
    fn test_warp_paths() {
        assert_eq!(warp::ACCOUNT_FILE, "/etc/wwps/wwps-core/warp_account.json");
        assert_eq!(
            warp::ROUTING_FILE,
            "/etc/wwps/wwps-core/conf/10_warp_routing.json"
        );
    }

    #[test]
    fn test_auto_update_paths() {
        assert_eq!(
            maintenance::UNATTENDED_UPGRADES_CONF,
            "/etc/apt/apt.conf.d/50unattended-upgrades"
        );
        assert_eq!(maintenance::DNF_AUTOMATIC_CONF, "/etc/dnf/automatic.conf");
        assert_eq!(
            maintenance::REBOOT_REQUIRED_FLAG,
            "/var/run/reboot-required"
        );
    }

    #[test]
    fn test_auto_update_supplementary_paths() {
        assert_eq!(
            maintenance::AUTO_UPGRADES_PERIODIC_CONF,
            "/etc/apt/apt.conf.d/20auto-upgrades"
        );
        assert_eq!(
            maintenance::NEEDRESTART_CONF,
            "/etc/needrestart/needrestart.conf"
        );
    }

    #[test]
    fn test_log_paths() {
        assert_eq!(log::DIR, "/var/log");
    }

    #[test]
    fn test_sub_server_paths() {
        assert_eq!(sub_server::BIN, "/usr/local/bin/sub-server");
        assert_eq!(sub_server::GRPC_SOCK, "/var/run/aegis/sub.sock");
        assert_eq!(sub_server::SERVICE, "wwps-sub-server");
        assert_eq!(
            sub_server::TLS_CERT,
            "/etc/wwps/sub-server/certs/fullchain.pem"
        );
    }
}
