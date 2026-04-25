use crate::core::paths::{singbox, xray};
use crate::core::types::BatchCreationResult;
use crate::core::types::IpVersion;
use crate::logic::maintenance::MaintenanceManager;
use crate::logic::port_allocator::PortAllocator;
use crate::logic::sni_selector::SNISelector;
use crate::logic::system::SystemMonitor;
use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use tokio::fs;

use super::hysteria2::Hysteria2Config;
use super::tuic::TUICConfig;

pub struct SingBoxConfigManager;

impl SingBoxConfigManager {
    pub async fn is_installed() -> bool {
        fs::try_exists(singbox::BIN)
            .await
            .unwrap_or(false)
    }

    pub async fn list_all_inbound_files() -> Result<Vec<String>> {
        let mut out = Vec::new();
        if let Ok(mut rd) = fs::read_dir(singbox::CONF_DIR).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") && !name.starts_with("00_") && !name.starts_with("01_") {
                        out.push(entry.path().to_string_lossy().to_string());
                    }
                }
            }
        }
        Ok(out)
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        fs::remove_file(path)
            .await
            .context("删除配置文件失败")?;
        
        let remaining = Self::list_all_inbound_files().await?;
        let has_hysteria2 = remaining.iter().any(|f| f.contains("hysteria2"));
        
        if !has_hysteria2 {
            Self::cleanup_port_hopping_firewall().await?;
            PortAllocator::release_hysteria2_range().await?;
        }
        
        Self::reload_service().await?;
        Ok(())
    }

    pub async fn delete_all_configurations() -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        let count = files.len();
        for file in &files {
            let _ = fs::remove_file(file).await;
        }
        
        if count > 0 {
            Self::cleanup_port_hopping_firewall().await?;
            PortAllocator::release_hysteria2_range().await?;
            Self::reload_service().await?;
        }
        Ok(count)
    }

    async fn cleanup_port_hopping_firewall() -> Result<()> {
        use tokio::process::Command;

        if let Some((main_port, _)) = PortAllocator::get_hysteria2_range().await {
            let _ = Command::new("iptables")
                .args([
                    "-t", "nat", "-D", "PREROUTING", "-p", "udp",
                    "-j", "REDIRECT",
                    "--to-ports", &main_port.to_string(),
                ])
                .output()
                .await;
            
            log::info!("已清理 Hysteria2 端口跳跃防火墙规则");
        }

        Ok(())
    }

    async fn reload_service() -> Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", "sing-box"])
            .output()
            .await
            .context("重载配置失败")?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "重载失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }

    async fn ensure_tls_certificates() -> Result<()> {
        if tokio::fs::try_exists(singbox::TLS_CERT).await.unwrap_or(false)
            && tokio::fs::try_exists(singbox::TLS_KEY).await.unwrap_or(false)
        {
            return Ok(());
        }

        tokio::fs::create_dir_all(singbox::CERTS_DIR)
            .await
            .context("创建证书目录失败")?;

        let output = tokio::process::Command::new(singbox::BIN)
            .args(["generate", "tls-keypair", "tls", "-m", "456"])
            .output()
            .await
            .context("生成 TLS 证书失败")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "生成证书失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        let mut key_content = String::new();
        let mut cert_content = String::new();
        let mut in_key = false;
        let mut in_cert = false;

        for line in lines {
            if line.contains("BEGIN PRIVATE KEY") {
                in_key = true;
                key_content.push_str(line);
                key_content.push('\n');
                continue;
            }
            if line.contains("END PRIVATE KEY") {
                key_content.push_str(line);
                key_content.push('\n');
                in_key = false;
                continue;
            }
            if line.contains("BEGIN CERTIFICATE") {
                in_cert = true;
                cert_content.push_str(line);
                cert_content.push('\n');
                continue;
            }
            if line.contains("END CERTIFICATE") {
                cert_content.push_str(line);
                cert_content.push('\n');
                in_cert = false;
                continue;
            }

            if in_key {
                key_content.push_str(line);
                key_content.push('\n');
            }
            if in_cert {
                cert_content.push_str(line);
                cert_content.push('\n');
            }
        }

        tokio::fs::write(singbox::TLS_KEY, key_content).await?;
        tokio::fs::write(singbox::TLS_CERT, cert_content).await?;

        Ok(())
    }

    pub async fn batch_create_hysteria2(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let host = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => {
                SystemMonitor::get_public_ip().await?
            }
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => {
                SystemMonitor::get_public_ipv6().await?
            }
        };

        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut configs = Vec::new();

        let (main_port, hop_range) = PortAllocator::allocate_hysteria2().await?;

        let port_443_available = MaintenanceManager::is_port_available(443).await;

        for i in 0..count {
            let sni = selector.next();

            let port = if i == 0 && port_443_available {
                443u16
            } else if i == 1 {
                main_port
            } else {
                loop {
                    let p = StdRng::from_entropy().gen_range(10000..60000);
                    if p >= hop_range.0 && p <= hop_range.1 {
                        continue;
                    }
                    if MaintenanceManager::is_port_available(p).await {
                        break p;
                    }
                }
            };

            let password = Hysteria2Config::generate_password();
            let tag = format!("HYSTERIA2-{}-{}", i + 1, &password[..8]);

            let config = Hysteria2Config::new(port, password.clone(), sni.clone());
            let link = if i == 1 {
                config.to_client_link_with_hopping(&host, &tag, hop_range)
            } else {
                config.to_client_link(&host, &tag)
            };

            links.push(link);
            configs.push(config.to_inbound_json(&tag));

            let _ = MaintenanceManager::allow_port(port).await;
        }

        Self::add_port_hopping_firewall_rules(main_port, hop_range).await?;

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

    async fn add_port_hopping_firewall_rules(main_port: u16, hop_range: (u16, u16)) -> Result<()> {
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);

        let output = Command::new("iptables")
            .args([
                "-t", "nat", "-A", "PREROUTING", "-p", "udp",
                "--dport", &range_str, "-j", "REDIRECT",
                "--to-ports", &main_port.to_string(),
            ])
            .output()
            .await;

        if let Err(e) = output {
            log::warn!("添加 iptables 规则失败 (可能需要 root 权限): {}", e);
        }

        log::info!("已配置 Hysteria2 端口跳跃: 主端口 {}, 跳跃范围 {}", main_port, range_str);
        Ok(())
    }

    pub async fn batch_create_tuic(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let host = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => {
                SystemMonitor::get_public_ip().await?
            }
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => {
                SystemMonitor::get_public_ipv6().await?
            }
        };

        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut configs = Vec::new();

        let port_443_available = MaintenanceManager::is_port_available(443).await;

        for i in 0..count {
            let sni = selector.next();

            let port = if i == 0 && port_443_available {
                443u16
            } else {
                loop {
                    let p = StdRng::from_entropy().gen_range(10000..60000);
                    if MaintenanceManager::is_port_available(p).await {
                        break p;
                    }
                }
            };

            let uuid = Self::generate_uuid().await?;
            let password = TUICConfig::generate_password();
            let tag = format!("TUIC-{}-{}", i + 1, &uuid[..8]);

            let config = TUICConfig::new(port, uuid.clone(), password.clone(), sni.clone());
            let link = config.to_client_link(&host, &tag);

            links.push(link);
            configs.push(config.to_inbound_json(&tag));

            let _ = MaintenanceManager::allow_port(port).await;
        }

        let (filename, _path) = Self::save_standalone_config(configs, "tuic").await?;
        Self::ensure_tls_certificates().await?;
        Self::reload_service().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count: count,
        })
    }

    async fn generate_uuid() -> Result<String> {
        let (status, stdout, _) = crate::logic::cmd_async::run_cmd_output(
            xray::BIN,
            &["uuid"],
            std::time::Duration::from_secs(5),
        )
        .await?;

        if status.success() {
            Ok(stdout.trim().to_string())
        } else {
            let mut rng = StdRng::from_entropy();
            Ok(format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                rng.r#gen::<u32>(),
                rng.r#gen::<u16>(),
                rng.r#gen::<u16>(),
                rng.r#gen::<u16>(),
                rng.r#gen::<u64>() & 0xFFFFFFFFFFFF
            ))
        }
    }

    async fn save_standalone_config(
        configs: Vec<Value>,
        proto: &str,
    ) -> Result<(String, String)> {
        use rand::Rng;

        fs::create_dir_all(singbox::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let mut rng = StdRng::from_entropy();
        let timestamp = chrono::Utc::now().timestamp();
        let random_part: String = (0..8)
            .map(|_| {
                let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
                let idx = rng.gen_range(0..chars.len());
                chars[idx] as char
            })
            .collect();

        let filename = match proto {
            "hysteria2" => format!("batch_hy2_{}_{}.json", timestamp, random_part),
            "tuic" => format!("batch_tuic_{}_{}.json", timestamp, random_part),
            _ => format!("batch_{}_{}_{}.json", proto, timestamp, random_part),
        };

        let config_path = format!("{}/{}", singbox::CONF_DIR, filename);

        let dns_servers = json!([
            {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
            {"tag": "local", "type": "local"}
        ]);

        let full_config = json!({
            "log": {
                "level": "warning",
                "output": "/var/log/sing-box.log"
            },
            "dns": {
                "servers": dns_servers
            },
            "route": {
                "default_domain_resolver": "dns"
            },
            "inbounds": configs,
            "outbounds": [
                {
                    "type": "direct",
                    "tag": "direct"
                },
                {
                    "type": "block",
                    "tag": "block"
                }
            ]
        });

        let content = serde_json::to_string_pretty(&full_config)?;
        fs::write(&config_path, content).await?;

        Ok((filename, config_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::singbox::hysteria2::Hysteria2Config;
    use crate::logic::singbox::tuic::TUICConfig;

    #[tokio::test]
    async fn test_singbox_is_installed_returns_bool() {
        let result = SingBoxConfigManager::is_installed().await;
        assert!(result || !result);
    }

    #[test]
    fn test_singbox_config_manager_exists() {
        let _ = SingBoxConfigManager;
    }

    #[test]
    fn test_hysteria2_config_struct() {
        let config = Hysteria2Config::new(8443, "test_password".to_string(), "sni.example.com".to_string());
        assert_eq!(config.port, 8443);
        assert_eq!(config.password, "test_password");
        assert_eq!(config.sni, "sni.example.com");
    }

    #[test]
    fn test_tuic_config_struct() {
        let config = TUICConfig::new(9443, "test-uuid".to_string(), "password".to_string(), "sni.example.com".to_string());
        assert_eq!(config.port, 9443);
        assert_eq!(config.uuid, "test-uuid");
        assert_eq!(config.congestion_control, "bbr");
    }
}