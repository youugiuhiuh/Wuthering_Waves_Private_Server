use anyhow::Result;
use rand::SeedableRng;
use rand::rngs::StdRng;

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

        let mut selector = crate::core::sni::selector::SNISelector::get_for_country(&country_code).await;

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
}
