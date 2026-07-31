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
        prefer_443: bool,
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

            let preferred = if prefer_443 && i == 0 && port_443_available {
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

        let provider = AcmeManager::configured_provider_for_domain(domain)?;
        let cdn_ports: &[u16] = provider.as_ref().map(|p| p.cdn_ports()).unwrap_or_default();

        if cdn_ports.is_empty() {
            anyhow::bail!(
                "no CDN provider configured for domain: no Cloudflare or Route53 credentials found"
            );
        }

        let mut rng = StdRng::from_entropy();
        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for (i, &cdn_port) in cdn_ports.iter().enumerate() {
            let port: i32 =
                if crate::core::system::maintenance::MaintenanceManager::is_port_available(cdn_port)
                    .await
                {
                    cdn_port as i32
                } else {
                    loop {
                        let p = rng.gen_range(10000..60000);
                        if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                            continue;
                        }
                        if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                            p,
                        )
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
                crate::core::system::maintenance::MaintenanceManager::allow_port(cdn_port).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, Proto::XHTTP).await
    }
}

#[cfg(test)]
mod tests {
    use crate::core::security::acme::CertPaths;
    use crate::core::types::{DnsProvider, IpVersion};
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

    #[test]
    fn cloudflare_cdn_ports_each_build_valid_node() {
        let certs = CertPaths {
            fullchain: "full.pem".into(),
            privkey: "key.pem".into(),
        };
        let cdn_ports = DnsProvider::Cloudflare.cdn_ports();
        assert_eq!(cdn_ports.len(), 6, "Cloudflare should have 6 CDN ports");
        for (i, &port) in cdn_ports.iter().enumerate() {
            let (config, link) = ConfigManager::build_tls_xhttp_node(
                i,
                port as i32,
                "uuid",
                "cdn-test.example.com",
                &certs,
                IpVersion::IPv4,
                "/cdn_test",
            );
            assert_eq!(config["port"], port, "port mismatch for index {i}");
            assert!(
                link.contains("security=tls"),
                "link missing tls for port {port}"
            );
            assert!(
                link.contains("@cdn-test.example.com:"),
                "link missing domain for port {port}"
            );
        }
    }

    #[test]
    fn route53_cdn_ports_contains_443() {
        let ports = DnsProvider::Route53.cdn_ports();
        assert_eq!(ports, &[443]);
        assert!(!ports.is_empty());
    }
}
