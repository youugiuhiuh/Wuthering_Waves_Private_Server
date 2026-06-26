use crate::core::paths::singbox;
use crate::core::paths::xray;
use crate::core::sni::selector::SNISelector;
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::types::{BatchCreationResult, IpVersion};
use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::config::SingBoxConfigManager;
use super::tuic::TUICConfig;

impl SingBoxConfigManager {
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

        let geoip = crate::core::network::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut configs = Vec::new();

        let port_443_available = MaintenanceManager::is_port_available(443).await;

        let cert_sha256 = SingBoxConfigManager::compute_cert_sha256_pin(singbox::TLS_CERT).await?;

        for i in 0..count {
            let sni = selector.get_next();

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

            let config = TUICConfig::new(port, uuid.clone(), password.clone(), sni.clone())
                .with_cert_sha256(cert_sha256.clone());
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
        let (status, stdout, _) = crate::core::cmd_async::run_cmd_output(
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
}
