use anyhow::{Result, anyhow};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};

use super::config::ConfigManager;
use super::kcp_mask::KcpMask;
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
                "security": "none",
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

        format!(
            "vless://{}@{}:{}?encryption=none&type=kcp&security=none&fm={}#{}",
            uuid, fmt_host, port, fm_encoded, encoded_email
        )
    }

    pub async fn batch_create_kcp(
        count: usize,
        ip_version: crate::core::types::IpVersion,
        mask_codes: &[&str],
    ) -> Result<BatchCreationResult> {
        let masks = KcpMask::parse_codes(mask_codes).map_err(|e| anyhow!("{}", e))?;

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
                    .await?
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

            let link =
                Self::generate_kcp_client_link(&uuid, &host, port, &email, ip_version, &masks);
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
    use crate::core::types::IpVersion;
    use serde_json::json;
    use std::process::Command;

    fn xray_available() -> bool {
        Command::new("xray")
            .arg("version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn validate_inbound(config: &Value, name: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join(format!("kcp_test_{name}.json"));
        let full = json!({"inbounds": [config]});
        let file = std::fs::File::create(&config_path).unwrap();
        serde_json::to_writer_pretty(file, &full).unwrap();

        let output = Command::new("xray")
            .args(["convert", "pb", "-outpbfile", "/dev/null"])
            .arg(config_path.to_str().unwrap())
            .output()
            .expect("xray convert pb failed");

        assert!(
            output.status.success(),
            "xray rejected mask '{name}': {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        );
    }

    #[test]
    fn test_kcp_finalmask_xray_available() {
        assert!(
            xray_available(),
            "xray not found — install xray-core v26.6.1+"
        );
    }

    #[test]
    fn test_kcp_finalmask_mkcp_legacy_with_xray() {
        if !xray_available() {
            return;
        }

        for (variant, header, value) in [
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
            validate_inbound(&config, &format!("mkcp-legacy-{variant}"));
        }
    }

    #[test]
    fn test_kcp_finalmask_noise_with_xray() {
        if !xray_available() {
            return;
        }

        let config = ConfigManager::build_kcp_inbound(
            "test",
            9999,
            "550e8400-e29b-41d4-a716-446655440000",
            "test@test",
            IpVersion::IPv4,
            &[KcpMask::Noise],
        );
        validate_inbound(&config, "noise");
    }

    #[test]
    fn test_kcp_finalmask_salamander_with_xray() {
        if !xray_available() {
            return;
        }

        for (variant, packet_size) in [
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
            validate_inbound(&config, &format!("salamander-{variant}"));
        }
    }

    #[test]
    fn test_kcp_finalmask_sudoku_with_xray() {
        if !xray_available() {
            return;
        }

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
        validate_inbound(&config, "sudoku");
    }

    #[test]
    fn test_kcp_finalmask_xdns_with_xray() {
        if !xray_available() {
            return;
        }

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
        validate_inbound(&config, "xdns");
    }

    #[test]
    fn test_kcp_finalmask_xicmp_with_xray() {
        if !xray_available() {
            return;
        }

        for (variant, dgram, ips) in [
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
            validate_inbound(&config, &format!("xicmp-{variant}"));
        }
    }

    #[test]
    fn test_kcp_finalmask_realm_with_xray() {
        if !xray_available() {
            return;
        }

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
        validate_inbound(&config, "realm");
    }

    #[test]
    fn test_kcp_finalmask_combined_with_xray() {
        if !xray_available() {
            return;
        }

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
        validate_inbound(&config, "combined");
    }
}
