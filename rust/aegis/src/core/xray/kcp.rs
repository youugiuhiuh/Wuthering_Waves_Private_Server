use anyhow::{Result, anyhow};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};

use super::config::ConfigManager;
use super::kcp_mask::KcpMask;
use crate::core::paths::singbox;
use crate::core::singbox::config::SingBoxConfigManager;
use crate::core::types::BatchCreationResult;

impl ConfigManager {
    pub(crate) fn build_kcp_inbound(
        tag: &str,
        port: i32,
        uuid: &str,
        email: &str,
        ip_version: crate::core::types::IpVersion,
        masks: &[KcpMask],
    ) -> Value {
        let listen_ip = match ip_version {
            crate::core::types::IpVersion::IPv4
            | crate::core::types::IpVersion::SplitStackV4Primary => "0.0.0.0",
            crate::core::types::IpVersion::IPv6
            | crate::core::types::IpVersion::SplitStackV6Primary => "::",
        };

        let client = json!({
            "id": uuid,
            "email": email
        });

        let udp_array: Vec<Value> = masks.iter().map(|m| m.as_json()).collect();

        json!({
            "listen": listen_ip,
            "port": port,
            "protocol": "vless",
            "tag": tag,
            "settings": {
                "clients": [client],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "kcp",
                "security": "tls",
                "tlsSettings": {
                    "certificates": [{
                        "certificateFile": singbox::TLS_CERT,
                        "keyFile": singbox::TLS_KEY
                    }]
                },
                "finalmask": {
                    "udp": udp_array
                },
                "kcpSettings": {
                    "mtu": 1350,
                    "tti": 50,
                    "uplinkCapacity": 5,
                    "downlinkCapacity": 20,
                    "cwndMultiplier": 1,
                    "maxSendingWindow": 2097152
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "metadataOnly": false
            }
        })
    }

    pub(crate) fn generate_kcp_client_link(
        uuid: &str,
        host: &str,
        port: i32,
        email: &str,
        ip_version: crate::core::types::IpVersion,
        masks: &[KcpMask],
        pin: &str,
    ) -> String {
        let udp_array: Vec<Value> = masks.iter().map(|m| m.as_json()).collect();
        let finalmask_json = json!({"udp": udp_array});
        let fm_str = serde_json::to_string(&finalmask_json).unwrap();
        let fm_encoded = utf8_percent_encode(&fm_str, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            crate::core::types::IpVersion::IPv6
            | crate::core::types::IpVersion::SplitStackV6Primary => format!("[{}]", host),
            crate::core::types::IpVersion::IPv4
            | crate::core::types::IpVersion::SplitStackV4Primary => host.to_string(),
        };
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();
        let encoded_pin = utf8_percent_encode(pin, NON_ALPHANUMERIC).to_string();

        format!(
            "vless://{}@{}:{}?encryption=none&type=kcp&security=tls&pcs={}&fm={}#{}",
            uuid, fmt_host, port, encoded_pin, fm_encoded, encoded_email
        )
    }

    pub async fn batch_create_kcp(
        count: usize,
        ip_version: crate::core::types::IpVersion,
        mask_codes: &[&str],
    ) -> Result<BatchCreationResult> {
        let masks = KcpMask::parse_codes(mask_codes).map_err(|e| anyhow!("{}", e))?;
        SingBoxConfigManager::ensure_tls_certificates().await?;
        let cert_pin = SingBoxConfigManager::compute_cert_sha256_pin(singbox::TLS_CERT).await?;

        let mask_types: Vec<&str> = masks.iter().map(|m| m.type_str()).collect();
        let mask_label = mask_types.join("+");

        let (ip, ipv6) = tokio::join!(
            crate::core::system::SystemMonitor::get_public_ip(),
            crate::core::system::SystemMonitor::get_public_ipv6(),
        );
        let (host, _) = ConfigManager::resolve_public_hosts(ip_version, ip, ipv6)?;

        let mut rng = StdRng::from_entropy();

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..count {
            let port = loop {
                let p = rng.gen_range(10000..60000);
                if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p)
                    .await
                {
                    continue;
                }
                if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await
                {
                    break p as i32;
                }
            };

            let uuid = ConfigManager::generate_wwps_uuid().await?;
            let uuid_short = ConfigManager::uuid_short_prefix(&uuid);

            let email = format!("{}-vless-kcp-{}", uuid_short, mask_label);
            let tag = format!("KCP-{}-{}", i + 1, uuid_short);

            let config = Self::build_kcp_inbound(&tag, port, &uuid, &email, ip_version, &masks);
            batch_configs.push(config);

            let link = Self::generate_kcp_client_link(
                &uuid, &host, port, &email, ip_version, &masks, &cert_pin,
            );
            links.push(link);

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, super::config::Proto::Kcp)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paths::singbox;
    use crate::core::types::IpVersion;
    use serde_json::json;
    fn assert_finalmask(config: &Value, expected: Value) {
        assert_eq!(
            config["streamSettings"]["finalmask"]["udp"],
            json!([expected])
        );
    }

    fn expected_mask(mask_type: &str, settings: Value) -> Value {
        let mut mask = json!({"type": mask_type});
        if settings != json!({}) {
            mask["settings"] = settings;
        }
        mask
    }

    #[test]
    fn test_kcp_inbound_always_uses_shared_tls_certificate() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Noise],
        );

        assert_eq!(config["streamSettings"]["security"], "tls");
        assert_eq!(
            config["streamSettings"]["tlsSettings"]["certificates"][0]["certificateFile"],
            singbox::TLS_CERT,
        );
        assert_eq!(
            config["streamSettings"]["tlsSettings"]["certificates"][0]["keyFile"],
            singbox::TLS_KEY,
        );
    }

    #[test]
    fn test_kcp_link_uses_percent_encoded_pinned_tls_without_insecure_bypass() {
        let link = ConfigManager::generate_kcp_client_link(
            "550e8400-e29b-41d4-a716-446655440000",
            "203.0.113.1",
            9999,
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Noise],
            "AA:BB:CC:DD",
        );

        assert!(link.contains("type=kcp"));
        assert!(link.contains("security=tls"));
        assert!(link.contains("pcs=AA%3ABB%3ACC%3ADD"));
        assert!(!link.contains("allowInsecure"));
        assert!(!link.contains("security=none"));
        assert!(!link.contains("vcn"));
    }

    #[test]
    fn test_kcp_finalmask_mkcp_legacy() {
        for (_variant, header, value) in [
            ("header+dns+value", Some("dns"), Some("example.com")),
            ("header+wechat+value", Some("wechat"), Some("123456")),
            ("value-only", None, Some("pwd")),
            ("no-settings", None, None),
        ] {
            let mask = KcpMask::MkcpLegacy {
                header: header.map(String::from),
                value: value.map(String::from),
            };
            let config = ConfigManager::build_kcp_inbound(
                "test",
                9999,
                "550e8400-e29b-41d4-a716-446655440000",
                "test@test",
                IpVersion::IPv4,
                &[mask],
            );
            let settings = match (header, value) {
                (Some(header), Some(value)) => json!({"header": header, "value": value}),
                (None, Some(value)) => json!({"value": value}),
                (None, None) => json!({}),
                (Some(header), None) => json!({"header": header}),
            };
            assert_finalmask(&config, expected_mask("mkcp-legacy", settings));
        }
    }

    #[test]
    fn test_kcp_finalmask_noise() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Noise],
        );
        assert_finalmask(&config, json!({"type": "noise"}));
    }

    #[test]
    fn test_kcp_finalmask_salamander() {
        for (_variant, packet_size) in [
            ("with-packet-size", Some((512, 1200))),
            ("no-packet-size", None),
        ] {
            let mask = KcpMask::Salamander {
                password: "test".into(),
                packet_size,
            };
            let config = ConfigManager::build_kcp_inbound(
                "test",
                9999,
                "550e8400-e29b-41d4-a716-446655440000",
                "test@test",
                IpVersion::IPv4,
                &[mask],
            );
            let settings = match packet_size {
                Some((min, max)) => json!({
                    "password": "test",
                    "packetSize": format!("{min}-{max}"),
                }),
                None => json!({"password": "test"}),
            };
            assert_finalmask(&config, expected_mask("salamander", settings));
        }
    }

    #[test]
    fn test_kcp_finalmask_sudoku() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Sudoku {
                password: "test".into(),
            }],
        );
        assert_finalmask(
            &config,
            json!({"type": "sudoku", "settings": {"password": "test"}}),
        );
    }

    #[test]
    fn test_kcp_finalmask_xdns() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Xdns {
                domains: vec!["example.com".into()],
                resolvers: vec!["example.com+udp://8.8.8.8:53".into()],
            }],
        );
        assert_finalmask(
            &config,
            json!({
                "type": "xdns",
                "settings": {
                    "domains": ["example.com"],
                    "resolvers": ["example.com+udp://8.8.8.8:53"],
                },
            }),
        );
    }

    #[test]
    fn test_kcp_finalmask_xicmp() {
        for (_variant, dgram, ips) in [
            ("dgram+ips", true, &["1.2.3.4", "5.6.7.8"] as &[&str]),
            ("no-settings", false, &[] as &[&str]),
        ] {
            let config = ConfigManager::build_kcp_inbound(
                "test",
                9999,
                "550e8400-e29b-41d4-a716-446655440000",
                "test@test",
                IpVersion::IPv4,
                &[KcpMask::Xicmp {
                    dgram,
                    ips: ips.iter().map(|s| s.to_string()).collect(),
                }],
            );
            let settings = if dgram || !ips.is_empty() {
                json!({"dgram": dgram, "ips": ips})
            } else {
                json!({})
            };
            assert_finalmask(&config, expected_mask("xicmp", settings));
        }
    }

    #[test]
    fn test_kcp_finalmask_realm() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Realm {
                url: "realm://token@example.com:443/id".into(),
                stun_servers: vec!["stun.l.google.com:19302".into()],
            }],
        );
        assert_finalmask(
            &config,
            json!({
                "type": "realm",
                "settings": {
                    "url": "realm://token@example.com:443/id",
                    "stunServers": ["stun.l.google.com:19302"],
                },
            }),
        );
    }

    #[test]
    fn test_kcp_finalmask_combined() {
        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[
                KcpMask::MkcpLegacy {
                    header: Some("dns".into()),
                    value: Some("example.com".into()),
                },
                KcpMask::Noise,
                KcpMask::Salamander {
                    password: "test".into(),
                    packet_size: Some((512, 1200)),
                },
                KcpMask::Sudoku {
                    password: "test".into(),
                },
                KcpMask::Xicmp {
                    dgram: true,
                    ips: vec!["1.2.3.4".into()],
                },
                KcpMask::Realm {
                    url: "realm://token@example.com:443/id".into(),
                    stun_servers: vec!["stun.l.google.com:19302".into()],
                },
            ],
        );
        assert_eq!(
            config["streamSettings"]["finalmask"]["udp"],
            json!([
                {
                    "type": "mkcp-legacy",
                    "settings": {"header": "dns", "value": "example.com"},
                },
                {"type": "noise"},
                {
                    "type": "salamander",
                    "settings": {"password": "test", "packetSize": "512-1200"},
                },
                {"type": "sudoku", "settings": {"password": "test"}},
                {
                    "type": "xicmp",
                    "settings": {"dgram": true, "ips": ["1.2.3.4"]},
                },
                {
                    "type": "realm",
                    "settings": {
                        "url": "realm://token@example.com:443/id",
                        "stunServers": ["stun.l.google.com:19302"],
                    },
                },
            ])
        );
    }
}
