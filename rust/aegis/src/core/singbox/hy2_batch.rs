use crate::core::sni::selector::SNISelector;
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::core::xray::port_allocator::PortAllocator;
use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::config::SingBoxConfigManager;
use super::hysteria2::{Hy2LinkStyle, Hysteria2Config, Hysteria2ObfsType};
use crate::core::paths::singbox;

impl SingBoxConfigManager {
    pub async fn batch_create_hysteria2(
        count: usize,
        ip_version: IpVersion,
        obfs_type: Option<Hysteria2ObfsType>,
        enable_hopping: bool,
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

        let mut selector = SNISelector::get_for_country(&country_code).await;

        let mut links = Vec::new();
        let mut configs = Vec::new();

        Self::ensure_tls_certificates().await?;
        let pin_sha256 = SingBoxConfigManager::compute_cert_sha256_pin(singbox::TLS_CERT).await?;

        for i in 0..count {
            let sni = selector.get_next();

            let (main_port, hop_range) = if enable_hopping {
                PortAllocator::allocate_hysteria2().await?
            } else {
                let port = loop {
                    let p = StdRng::from_entropy().gen_range(10000..60000);
                    if PortAllocator::is_port_in_locked_range(p).await {
                        continue;
                    }
                    if MaintenanceManager::is_port_available(p).await {
                        break p;
                    }
                };
                (port, (port, port))
            };

            let password = Hysteria2Config::generate_password();
            let tag = format!("HYSTERIA2-{}-{}", i + 1, &password[..8]);

            let config = if let Some(obfs_type) = obfs_type {
                let obfs_password = Hysteria2Config::generate_obfs_password();
                Hysteria2Config::with_obfs(
                    main_port,
                    password.clone(),
                    sni.clone(),
                    obfs_type,
                    obfs_password,
                )
                .with_pin_sha256(pin_sha256.clone())
            } else {
                Hysteria2Config::new(main_port, password.clone(), sni.clone())
                    .with_pin_sha256(pin_sha256.clone())
            };

            let link = if obfs_type.is_some() && enable_hopping {
                config.to_client_link_with_hopping_and_obfs(
                    &host,
                    &tag,
                    hop_range,
                    Hy2LinkStyle::Official,
                )
            } else if obfs_type.is_some() {
                config.to_client_link_with_obfs(&host, &tag)
            } else if enable_hopping {
                config.to_client_link_with_hopping(&host, &tag, hop_range, Hy2LinkStyle::Official)
            } else {
                config.to_client_link(&host, &tag)
            };

            links.push(link);
            configs.push(config.to_inbound_json(&tag));

            let _ = MaintenanceManager::allow_port(main_port).await;

            if enable_hopping {
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
        }

        let (filename, _path) = Self::save_standalone_config(configs, "hysteria2").await?;
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
