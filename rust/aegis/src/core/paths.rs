//! 集中路径常量定义
//!
//! 所有系统路径在此定义，避免硬编码分散
//!
//! # 命名说明（部署改名映射）
//!
//! 上游代理二进制在部署时会被重命名，因此磁盘路径 / systemd 服务名 / 进程名
//! 与上游产品名不同：
//!
//! | 上游产品 | 部署名 | 安装目录 | systemd 服务 |
//! |----------|--------|----------|--------------|
//! | Xray-core | `wwps-core` | `/etc/wwps/wwps-core/` | `wwps-core` |
//! | Sing-box  | `wwps-box`  | `/etc/wwps/wwps-box/`  | `wwps-box`  |
//!
//! 即：**`wwps-core` = Xray-core，`wwps-box` = Sing-box**。
//!
//! - 面向用户的文案（i18n / 机器人消息 / 文档）一律使用上游产品名：Xray-core、Sing-box。
//! - 磁盘路径、systemd 服务名、进程名、i18n 键（如 `menu.wwps_core_mgmt`）与
//!   Rust 标识符（如 `WWPS_CORE_*`、`install_wwps_core`）保留 `wwps_*` 前缀——
//!   已有部署 / 升级脚本 / 防火墙规则依赖它们，禁止重命名。
//! - 命令注意：ML-DSA-65 密钥生成实际执行 `wwps-core mldsa65`（回退 `xray mldsa65`），
//!   两者指向同一个 Xray-core 二进制（见 `bootstrap.rs` 与 `core/xray/reality.rs`）。

pub const WWPS_BASE_DIR: &str = "/etc/wwps";

pub mod acme {
    pub const HOME: &str = "/root/.acme.sh";
    pub const BIN: &str = "/root/.acme.sh/acme.sh";
    pub const ACCOUNT_CONF: &str = "/root/.acme.sh/account.conf";
    pub const CERT_ROOT: &str = "/root/cert";
}

/// Sing-box 相关路径
pub mod singbox {
    pub const DIR: &str = "/etc/wwps/wwps-box";
    pub const BIN: &str = "/etc/wwps/wwps-box/wwps-box";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-box/conf";
    pub const CERTS_DIR: &str = "/etc/wwps/wwps-box/certs";
    pub const TLS_CERT: &str = "/etc/wwps/wwps-box/certs/tls.cer";
    pub const TLS_KEY: &str = "/etc/wwps/wwps-box/certs/tls.key";
    pub const RULE_SET_DIR: &str = "/etc/wwps/wwps-box/rule-set";
}

/// Bot 相关路径
pub mod bot {
    pub const DIR: &str = "/etc/wwps/aegis";
    pub const KEY_FILE: &str = "/etc/wwps/aegis/.key";
    pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/aegis/bbr3_pending.flag";

    /// SimpleX bot 的 long-term 连接地址。aegis 连接后写入，installer 读取并打印。
    /// 必须与 go/installer/main.go 的 simplexAddressFile 保持一致。
    ///
    /// 不可改动：文件名 `simplex_address` 是与 installer 共享的落盘契约，
    /// 改名会让存量部署的地址文件变成孤儿，admin 连接信息丢失。
    pub const SIMPLEX_ADDRESS_FILE: &str = "/etc/wwps/aegis/simplex_address";
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

    // ── 出口连通性探测（`core/network/warp_probe.rs`）──
    //
    // 常量集中在此，与 `ACCOUNT_FILE` 风格一致；同时让探测目标可在测试中
    // 被显式断言（而不是散落在实现里成为魔法值）。

    /// wg0 的累计接收字节数。
    ///
    /// Xray 的 `proxy/wireguard` 在 Linux 上以 kernel TUN 模式建 wg0，
    /// 该计数器恒为 0 即代表「握手包发得出去、回包收不到」。
    pub const WG_RX_BYTES_PATH: &str = "/sys/class/net/wg0/statistics/rx_bytes";

    /// WireGuard 握手探测目标（Cloudflare WARP 消费级端点）。
    pub const UDP_PROBE_ENDPOINT: &str = "162.159.192.1:2408";

    /// MASQUE over HTTP/2（TCP/443）探测目标。
    ///
    /// 取 Cloudflare 官方文档中 **MASQUE** 专用段 `162.159.197.0/24` 的首地址。
    /// 选它而非 usque 硬编码的 `162.159.198.2`：后者真机实测 **不**协商 h2。
    ///
    /// 真机实测（2026-10-14，racknerd-534f8f8，TLS1.3）：
    ///
    /// - `162.159.197.1` → ALPN=`h2` ✅（MASQUE 段）
    /// - `162.159.192.1` → ALPN=`h2` ✅（消费级 WARP 段）
    /// - `162.159.198.2` → ALPN=`None` ❌（usque 默认值，此处不可用）
    ///
    /// 判定：TCP 通且 ALPN 协商出 `h2` 即代表「出口 UDP 被封时仍可走
    /// TCP/443」，这是后续 usque 集成（独立 Spec 2）的可行性前提。
    pub const TCP_PROBE_ENDPOINT: &str = "162.159.197.1:443";

    /// 上述探测使用的 SNI。
    pub const TCP_PROBE_SNI: &str = "consumer-masque.cloudflareclient.com";
}

/// 日志目录
pub mod log {
    pub const DIR: &str = "/var/log";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acme_paths() {
        assert_eq!(acme::HOME, "/root/.acme.sh");
        assert_eq!(acme::BIN, "/root/.acme.sh/acme.sh");
        assert_eq!(acme::ACCOUNT_CONF, "/root/.acme.sh/account.conf");
        assert_eq!(acme::CERT_ROOT, "/root/cert");
    }

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
        assert_eq!(singbox::RULE_SET_DIR, "/etc/wwps/wwps-box/rule-set"); // 新增
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

    /// 探测常量是**实测得出**的目标，不是随手填的值。
    ///
    /// 真机证据（2026-10-14，racknerd-534f8f8）：
    ///
    /// - 出站 UDP 2408/500/1701/4500 → 全部静默（被上游丢弃）
    /// - 出站 UDP 53 → 正常（证明 UDP 出站本身可用）
    /// - TCP 162.159.197.1:443 → 可连且 ALPN 协商出 h2
    /// - TCP 162.159.198.2:443 → 可连但 **ALPN=None**（故未被选用）
    ///
    /// 这条测试把上述目标钉死，防止有人「顺手」改成一个不可用的端点。
    #[test]
    fn test_warp_probe_constants() {
        assert_eq!(
            warp::WG_RX_BYTES_PATH,
            "/sys/class/net/wg0/statistics/rx_bytes"
        );
        assert_eq!(warp::UDP_PROBE_ENDPOINT, "162.159.192.1:2408");
        assert_eq!(warp::TCP_PROBE_ENDPOINT, "162.159.197.1:443");
        assert_eq!(warp::TCP_PROBE_SNI, "consumer-masque.cloudflareclient.com");
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
}
