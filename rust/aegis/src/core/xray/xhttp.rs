use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

use super::config::{ConfigManager, Proto};
use crate::core::security::acme::AcmeManager;
use crate::core::security::acme::CertPaths;
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
        domain: &str,
        certs: &CertPaths,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let _ = AcmeManager::validate_domain(domain)?;

        let port_443_available =
            crate::core::system::maintenance::MaintenanceManager::is_port_available(443).await;

        let mut rng = StdRng::from_entropy();
        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..20 {
            let port: i32 = if i == 0 && port_443_available {
                443
            } else {
                loop {
                    let p = rng.gen_range(10000..60000);
                    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p)
                        .await
                    {
                        continue;
                    }
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(p)
                        .await
                    {
                        break p as i32;
                    }
                }
            };

            let uuid = ConfigManager::generate_wwps_uuid().await?;
            let path = ConfigManager::generate_random_path();

            let (config, link) = ConfigManager::build_tls_xhttp_node(
                i, port, &uuid, domain, certs, ip_version, &path,
            );

            batch_configs.push(config);
            links.push(link);

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, Proto::XHTTP).await
    }
}

#[cfg(test)]
mod tests {
    use crate::core::security::acme::CertPaths;
    use crate::core::types::IpVersion;
    use crate::core::xray::config::ConfigManager;

    #[test]
    fn build_tls_node_returns_matching_config_and_link() {
        let certs = CertPaths {
            fullchain: "full.pem".into(),
            privkey: "key.pem".into(),
        };
        let (config, link) = ConfigManager::build_tls_xhttp_node(
            0,
            2053,
            "uuid",
            "example.com",
            &certs,
            IpVersion::IPv4,
            "/xhttp_test",
        );
        assert_eq!(config["port"], 2053);
        assert!(link.contains("security=tls"));
        assert!(link.contains("host=example%2Ecom"));
    }
}
