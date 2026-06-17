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
                "udpmasks": udp_array,
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
        let fm_str = serde_json::to_string(&udp_array).unwrap();
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

        let (host, _) = ConfigManager::resolve_public_hosts(
            ip_version,
            crate::core::system::SystemMonitor::get_public_ip().await,
            crate::core::system::SystemMonitor::get_public_ipv6().await,
        )?;

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
