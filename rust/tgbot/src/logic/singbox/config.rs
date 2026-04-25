use crate::logic::config::IpVersion;
use crate::logic::maintenance::MaintenanceManager;
use crate::logic::sni_selector::SNISelector;
use crate::logic::system::SystemMonitor;
use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use tokio::fs;

use super::hysteria2::Hysteria2Config;
use super::tuic::TUICConfig;

const WWPS_BOX_DIR: &str = "/etc/wwps/wwps-box";
const WWPS_BOX_CONF_DIR: &str = "/etc/wwps/wwps-box/conf";
const WWPS_BOX_BIN: &str = "/etc/wwps/wwps-box/sing-box";

pub struct BatchCreationResult {
    pub links: Vec<String>,
    pub config_file: Option<String>,
    pub backup_file: Option<String>,
    pub created_count: usize,
}

pub struct SingBoxConfigManager;

impl SingBoxConfigManager {
    pub async fn is_installed() -> bool {
        fs::try_exists(WWPS_BOX_BIN)
            .await
            .unwrap_or(false)
    }

    pub async fn list_all_inbound_files() -> Result<Vec<String>> {
        let mut out = Vec::new();
        if let Ok(mut rd) = fs::read_dir(WWPS_BOX_CONF_DIR).await {
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
            Self::reload_service().await?;
        }
        Ok(count)
    }

    const WWPS_BOX_CERTS_DIR: &str = "/etc/wwps/wwps-box/certs";
    const WWPS_BOX_TLS_CERT: &str = "/etc/wwps/wwps-box/certs/tls.cer";
    const WWPS_BOX_TLS_KEY: &str = "/etc/wwps/wwps-box/certs/tls.key";

    async fn reload_service() -> Result<()> {
        let output = tokio::process::Command::new(WWPS_BOX_BIN)
            .args(["run", "-C", WWPS_BOX_CONF_DIR])
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
        if tokio::fs::try_exists(Self::WWPS_BOX_TLS_CERT).await.unwrap_or(false)
            && tokio::fs::try_exists(Self::WWPS_BOX_TLS_KEY).await.unwrap_or(false)
        {
            return Ok(());
        }

        tokio::fs::create_dir_all(Self::WWPS_BOX_CERTS_DIR)
            .await
            .context("创建证书目录失败")?;

        let output = tokio::process::Command::new(WWPS_BOX_BIN)
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
                in_cert = false;
            } else if line.contains("BEGIN CERTIFICATE") {
                in_cert = true;
                in_key = false;
            } else if line.contains("END PRIVATE KEY") {
                in_key = false;
            } else if line.contains("END CERTIFICATE") {
                in_cert = false;
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

        tokio::fs::write(Self::WWPS_BOX_TLS_KEY, key_content).await?;
        tokio::fs::write(Self::WWPS_BOX_TLS_CERT, cert_content).await?;

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

            let password = Hysteria2Config::generate_password();
            let tag = format!("HYSTERIA2-{}-{}", i + 1, &password[..8]);

            let config = Hysteria2Config::new(port, password.clone(), sni.clone());
            let link = config.to_client_link(&host, &tag);

            links.push(link);
            configs.push(config.to_inbound_json(&tag));

            let _ = MaintenanceManager::allow_port(port).await;
        }

        Self::save_configs(configs).await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some("hysteria2_inbounds.json".to_string()),
            backup_file: None,
            created_count: count,
        })
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

        Self::save_configs(configs).await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some("tuic_inbounds.json".to_string()),
            backup_file: None,
            created_count: count,
        })
    }

    async fn generate_uuid() -> Result<String> {
        let (status, stdout, _) = crate::logic::cmd_async::run_cmd_output(
            "/etc/wwps/wwps-core/wwps-core",
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

    async fn save_configs(configs: Vec<Value>) -> Result<()> {
        fs::create_dir_all(WWPS_BOX_CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let config_path = format!("{}/02_singbox_inbounds.json", WWPS_BOX_CONF_DIR);

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

        Self::ensure_tls_certificates().await?;
        Self::reload_service().await?;

        Ok(())
    }
}