use anyhow::{Context, Result};
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::json;
use tokio::fs;

use super::config::ConfigManager;
use crate::core::types::{BatchCreationResult, IpVersion};

impl ConfigManager {
    pub async fn batch_create_xhttp_reality_enhanced(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let (ip, ipv6) = tokio::join!(
            crate::core::system::SystemMonitor::get_public_ip(),
            crate::core::system::SystemMonitor::get_public_ipv6(),
        );
        let (host, host_secondary) = ConfigManager::resolve_public_hosts(ip_version, ip, ipv6)?;

        let mut rng = StdRng::from_entropy();
        let geoip = crate::core::network::geoip::GeoIPService::new();

        let (country_code, port_443_available) = tokio::join!(
            geoip.get_country_code(),
            crate::core::system::maintenance::MaintenanceManager::is_port_available(443),
        );

        let mut selector =
            crate::core::sni::selector::SNISelector::get_for_country(&country_code).await;

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..count {
            let sni = selector.get_next();

            let pq_ok = crate::core::security::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                ConfigManager::generate_enhanced_config(
                    &mut rng,
                    sni,
                    i,
                    super::config::Proto::XHTTP,
                    preferred,
                )
                .await?;

            let config = ConfigManager::build_reality_vless_inbound(
                &tag,
                port,
                &uuid,
                &email,
                &sni,
                &pub_key,
                &priv_key,
                &short_id,
                ip_version,
                super::config::Proto::XHTTP,
                path.as_deref(),
                pq_ok,
            );

            batch_configs.push(config);

            let link = ConfigManager::generate_client_link(
                &uuid,
                &host,
                port,
                &sni,
                &pub_key,
                &short_id,
                &email,
                ip_version,
                super::config::Proto::XHTTP,
                path.as_deref(),
                host_secondary.as_deref(),
                pq_ok,
            );
            links.push(link);

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, super::config::Proto::XHTTP)
            .await
    }

    pub async fn batch_create_xhttp_tls_enhanced(
        count: usize,
        ip_version: IpVersion,
        domain: &str,
        cert_paths: &crate::core::security::acme::CertPaths,
    ) -> Result<BatchCreationResult> {
        let (ip, ipv6) = tokio::join!(
            crate::core::system::SystemMonitor::get_public_ip(),
            crate::core::system::SystemMonitor::get_public_ipv6(),
        );
        let (_host, host_secondary) = ConfigManager::resolve_public_hosts(ip_version, ip, ipv6)?;

        let mut rng = StdRng::from_entropy();

        let port_443_available =
            crate::core::system::maintenance::MaintenanceManager::is_port_available(443).await;

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        let cert_fullchain = cert_paths.fullchain.to_string_lossy().to_string();
        let cert_privkey = cert_paths.privkey.to_string_lossy().to_string();

        for i in 0..count {
            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, email, tag, path) =
                ConfigManager::generate_tls_xhttp_config(&mut rng, domain, i, preferred).await?;

            let config = ConfigManager::build_tls_xhttp_inbound(
                &tag,
                port,
                &uuid,
                &email,
                domain,
                &cert_fullchain,
                &cert_privkey,
                ip_version,
                Some(&path),
            );

            batch_configs.push(config);

            let link = ConfigManager::generate_client_link_tls(
                &uuid,
                domain,
                port,
                &email,
                ip_version,
                Some(&path),
                host_secondary.as_deref(),
            );
            links.push(link);

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        let filename = ConfigManager::generate_secure_batch_filename_tls().await?;
        let config_path = format!("{}/{}", crate::core::paths::xray::CONF_DIR, filename);

        let config = json!({ "inbounds": batch_configs });
        let content = serde_json::to_string_pretty(&config).context("序列化配置失败")?;
        fs::write(&config_path, content).await?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count: count,
        })
    }

    pub(crate) async fn generate_secure_batch_filename_tls() -> Result<String> {
        let uuid = ConfigManager::generate_wwps_uuid().await?;
        let uuid_short = ConfigManager::uuid_short_prefix(&uuid);
        Ok(format!("batch_xhttp_tls_{}_inbounds.json", uuid_short))
    }
}
