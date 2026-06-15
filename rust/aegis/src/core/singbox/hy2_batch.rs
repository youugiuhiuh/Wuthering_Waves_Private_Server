use crate::core::sni::selector::SNISelector;
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::core::xray::port_allocator::PortAllocator;
use anyhow::Result;

use super::config::SingBoxConfigManager;
use super::hysteria2::Hysteria2Config;

impl SingBoxConfigManager {
    pub async fn batch_create_hysteria2(
        count: usize,
        ip_version: IpVersion,
        enable_obfs: bool,
    ) -> Result<BatchCreationResult> {
        if !PortAllocator::check_hysteria2_limit().await? {
            return Err(anyhow::anyhow!("已达到最大 Hysteria2 配置数量限制（50个）"));
        }

        let host = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => {
                SystemMonitor::get_public_ip().await?
            }
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => {
                SystemMonitor::get_public_ipv6().await?
            }
        };

        let geoip = crate::core::network::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut configs = Vec::new();

        for i in 0..count {
            let sni = selector.get_next();

            let (main_port, hop_range) = PortAllocator::allocate_hysteria2().await?;

            let port = main_port;

            let password = Hysteria2Config::generate_password();
            let tag = format!("HYSTERIA2-{}-{}", i + 1, &password[..8]);

            let config = if enable_obfs {
                let obfs_password = Hysteria2Config::generate_obfs_password();
                Hysteria2Config::with_obfs(
                    port,
                    password.clone(),
                    sni.clone(),
                    "salamander".to_string(),
                    obfs_password,
                )
            } else {
                Hysteria2Config::new(port, password.clone(), sni.clone())
            };

            let link = if enable_obfs {
                config.to_client_link_with_hopping_and_obfs(&host, &tag, hop_range)
            } else {
                config.to_client_link_with_hopping(&host, &tag, hop_range)
            };

            links.push(link);
            configs.push(config.to_inbound_json(&tag));

            let _ = MaintenanceManager::allow_port(main_port).await;
            let _ = MaintenanceManager::allow_port_range(hop_range.0, hop_range.1).await;

            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            if has_ipv6 {
                let _ = MaintenanceManager::allow_port_range_v6(hop_range.0, hop_range.1).await;
            }

            Self::add_port_hopping_firewall_rules_v4(main_port, hop_range).await?;
            if has_ipv6 {
                Self::add_port_hopping_firewall_rules_v6(main_port, hop_range).await?;
            }
        }

        let (filename, _path) = Self::save_standalone_config(configs, "hysteria2").await?;
        Self::ensure_tls_certificates().await?;
        Self::reload_service().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count: count,
        })
    }

    async fn add_port_hopping_firewall_rules_v4(
        main_port: u16,
        hop_range: (u16, u16),
    ) -> Result<()> {
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);

        let output = Command::new("iptables")
            .args([
                "-t",
                "nat",
                "-A",
                "PREROUTING",
                "-p",
                "udp",
                "--dport",
                &range_str,
                "-j",
                "REDIRECT",
                "--to-ports",
                &main_port.to_string(),
            ])
            .output()
            .await;

        if let Err(e) = output {
            log::warn!("添加 iptables 规则失败 (可能需要 root 权限): {}", e);
        }

        log::info!(
            "已配置 Hysteria2 IPv4 端口跳跃: 主端口 {}, 跳跃范围 {}",
            main_port,
            range_str
        );
        Ok(())
    }

    async fn add_port_hopping_firewall_rules_v6(
        main_port: u16,
        hop_range: (u16, u16),
    ) -> Result<()> {
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);

        let output = Command::new("ip6tables")
            .args([
                "-t",
                "nat",
                "-A",
                "PREROUTING",
                "-p",
                "udp",
                "--dport",
                &range_str,
                "-j",
                "REDIRECT",
                "--to-ports",
                &main_port.to_string(),
            ])
            .output()
            .await;

        if let Err(e) = output {
            log::warn!("添加 ip6tables 规则失败 (可能需要 root 权限): {}", e);
        }

        log::info!(
            "已配置 Hysteria2 IPv6 端口跳跃: 主端口 {}, 跳跃范围 {}",
            main_port,
            range_str
        );
        Ok(())
    }
}
