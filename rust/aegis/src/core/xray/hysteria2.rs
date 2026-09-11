use anyhow::Result;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::types::{BatchCreationResult, IpVersion};

impl ConfigManager {
    /// Build a single Xray `hysteria` inbound (Hysteria2).
    ///
    /// Xray splits Hysteria2 configuration across two layers and BOTH must be
    /// present: the protocol layer (`protocol: "hysteria"` + `settings.users`)
    /// and the transport layer (`network: "hysteria"` + `hysteriaSettings`).
    /// Either alone produces an inbound that never authenticates.
    pub(crate) fn build_hysteria2_inbound(
        tag: &str,
        port: i32,
        auth: &str,
        email: &str,
        sni: &str,
        ip_version: IpVersion,
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => "0.0.0.0",
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => "::",
        };

        json!({
            "listen": listen_ip,
            "port": port,
            "protocol": "hysteria",
            "tag": tag,
            "settings": {
                "version": 2,
                "users": [{
                    "auth": auth,
                    "level": 0,
                    "email": email
                }]
            },
            "streamSettings": {
                "network": "hysteria",
                "security": "tls",
                "tlsSettings": {
                    "serverName": sni,
                    "alpn": ["h3"],
                    "certificates": [{
                        "usage": "encipherment",
                        "certificateFile": singbox::TLS_CERT,
                        "keyFile": singbox::TLS_KEY
                    }]
                },
                "hysteriaSettings": {
                    "version": 2,
                    "auth": auth
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "metadataOnly": false
            }
        })
    }

    /// Build a `hysteria2://` share link for an Xray `hysteria` inbound.
    ///
    /// `auth` MUST be the same value used in `build_hysteria2_inbound`'s
    /// `settings.users[].auth`, otherwise the client cannot authenticate.
    pub(crate) fn generate_hysteria2_client_link(
        auth: &str,
        host: &str,
        port: i32,
        sni: &str,
        email: &str,
        ip_version: IpVersion,
        pin: &str,
    ) -> String {
        let encoded_auth = utf8_percent_encode(auth, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();
        let encoded_pin = utf8_percent_encode(pin, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&pinSHA256={}#{}",
            encoded_auth, fmt_host, port, encoded_sni, encoded_pin, encoded_email
        )
    }

    /// Batch-create `count` Xray Hysteria2 inbounds with matching share links.
    ///
    /// Scope is deliberately core-only: no `finalmask.udp` obfuscation and no
    /// `quicParams.udpHop` port hopping. Both are additive follow-ups.
    pub async fn batch_create_hysteria2_xray(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        use crate::core::singbox::config::SingBoxConfigManager;

        if !crate::core::xray::port_allocator::PortAllocator::check_hysteria2_limit().await? {
            return Err(anyhow::anyhow!("已达到最大 Hysteria2 配置数量限制（50个）"));
        }

        // Shared certificate, same one sing-box Hy2 and Xray KCP use.
        SingBoxConfigManager::ensure_tls_certificates().await?;
        let pin = SingBoxConfigManager::compute_cert_sha256_pin(singbox::TLS_CERT).await?;

        let (ip, ipv6) = tokio::join!(
            crate::core::system::SystemMonitor::get_public_ip(),
            crate::core::system::SystemMonitor::get_public_ipv6(),
        );
        let (host, _) = Self::resolve_public_hosts(ip_version, ip, ipv6)?;

        let geoip = crate::core::network::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;
        let mut selector =
            crate::core::sni::selector::SNISelector::get_for_country(&country_code).await;

        let mut rng = StdRng::from_entropy();
        let mut links = Vec::with_capacity(count);
        let mut configs = Vec::with_capacity(count);

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

            // Reuse the existing 32-char alphanumeric generator rather than
            // duplicating it. Xray Hysteria2 authenticates with a password.
            let auth = crate::core::singbox::hysteria2::Hysteria2Config::generate_password();
            let uuid = Self::generate_wwps_uuid().await?;
            let uuid_short = Self::uuid_short_prefix(&uuid);
            let sni = selector.get_next();

            let email = format!("{}-hysteria2", uuid_short);
            let tag = format!("HY2-{}-{}", i + 1, uuid_short);

            configs.push(Self::build_hysteria2_inbound(
                &tag, port, &auth, &email, &sni, ip_version,
            ));

            links.push(Self::generate_hysteria2_client_link(
                &auth, &host, port, &sni, &email, ip_version, &pin,
            ));

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        Self::create_standalone_config(configs, links, super::config::Proto::Hysteria2).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH: &str = "TestAuth1234567890abcdefghijklmn";

    fn build(ip_version: IpVersion) -> Value {
        ConfigManager::build_hysteria2_inbound(
            "HY2-test-1",
            11451,
            AUTH,
            "abcd1234-hysteria2",
            "www.bing.com",
            ip_version,
        )
    }

    #[test]
    fn test_hysteria2_inbound_uses_xray_protocol_name_not_singbox() {
        let cfg = build(IpVersion::IPv4);
        // Xray protocol string is "hysteria"; sing-box's "hysteria2" is rejected.
        assert_eq!(cfg["protocol"], "hysteria");
        assert_ne!(cfg["protocol"], "hysteria2");
    }

    #[test]
    fn test_hysteria2_inbound_has_both_protocol_and_transport_layers() {
        let cfg = build(IpVersion::IPv4);
        // Both layers are mandatory; either alone never authenticates.
        assert_eq!(cfg["settings"]["version"], 2);
        assert_eq!(cfg["streamSettings"]["network"], "hysteria");
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["version"], 2);
    }

    #[test]
    fn test_hysteria2_inbound_auth_matches_across_both_layers() {
        let cfg = build(IpVersion::IPv4);
        // A mismatch between these two means clients cannot connect.
        assert_eq!(cfg["settings"]["users"][0]["auth"], AUTH);
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["auth"], AUTH);
    }

    #[test]
    fn test_hysteria2_inbound_uses_auth_not_password_field() {
        let cfg = build(IpVersion::IPv4);
        let user = &cfg["settings"]["users"][0];
        assert_eq!(user["auth"], AUTH);
        // sing-box uses users[].password; Xray must not.
        assert!(user.get("password").is_none());
    }

    #[test]
    fn test_hysteria2_inbound_alpn_is_explicitly_h3() {
        let cfg = build(IpVersion::IPv4);
        assert_eq!(cfg["streamSettings"]["security"], "tls");
        assert_eq!(cfg["streamSettings"]["tlsSettings"]["alpn"], json!(["h3"]));
        assert_eq!(
            cfg["streamSettings"]["tlsSettings"]["serverName"],
            "www.bing.com"
        );
    }

    #[test]
    fn test_hysteria2_inbound_reuses_shared_tls_certificate() {
        let cfg = build(IpVersion::IPv4);
        let cert = &cfg["streamSettings"]["tlsSettings"]["certificates"][0];
        assert_eq!(cert["certificateFile"], singbox::TLS_CERT);
        assert_eq!(cert["keyFile"], singbox::TLS_KEY);
    }

    #[test]
    fn test_hysteria2_inbound_listen_ip_per_version() {
        assert_eq!(build(IpVersion::IPv4)["listen"], "0.0.0.0");
        assert_eq!(build(IpVersion::SplitStackV4Primary)["listen"], "0.0.0.0");
        assert_eq!(build(IpVersion::IPv6)["listen"], "::");
        assert_eq!(build(IpVersion::SplitStackV6Primary)["listen"], "::");
    }

    #[test]
    fn test_hysteria2_inbound_records_tag_port_email() {
        let cfg = build(IpVersion::IPv4);
        assert_eq!(cfg["tag"], "HY2-test-1");
        assert_eq!(cfg["port"], 11451);
        assert_eq!(cfg["settings"]["users"][0]["email"], "abcd1234-hysteria2");
        assert_eq!(cfg["settings"]["users"][0]["level"], 0);
    }

    #[test]
    fn test_hysteria2_link_uses_hysteria2_uri_scheme() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "abcd-hysteria2",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        assert!(link.starts_with("hysteria2://pw123@203.0.113.1:11451?"));
        // The brief's own format spec mandates `<pct-encoded sni>` / `<pct-encoded email>`,
        // and NON_ALPHANUMERIC encodes `.` as %2E and `-` as %2D. Same encoding as the
        // shipped sing-box Hy2 link generator, so client behaviour is identical.
        assert!(link.contains("sni=www%2Ebing%2Ecom"));
        assert!(link.contains("alpn=h3"));
        assert!(link.ends_with("#abcd%2Dhysteria2"));
    }

    #[test]
    fn test_hysteria2_link_never_enables_insecure_bypass() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        assert!(!link.contains("insecure=1"));
        assert!(!link.contains("allowInsecure"));
        assert!(!link.contains("security=none"));
    }

    #[test]
    fn test_hysteria2_link_pins_certificate_with_percent_encoding() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        // Colons must be escaped so they do not terminate the query value.
        assert!(link.contains("pinSHA256=AA%3ABB%3ACC%3ADD"));
    }

    #[test]
    fn test_hysteria2_link_encodes_special_characters_in_auth() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "p@ss!word",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA",
        );
        assert!(link.contains("@203.0.113.1:11451"));
        assert!(link.contains("p%40ss%21word"));
        assert!(!link.contains("p@ss!word@"));
    }

    #[test]
    fn test_hysteria2_link_brackets_ipv6_host() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "2001:db8::1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv6,
            "AA",
        );
        assert!(link.contains("@[2001:db8::1]:11451?"));
    }

    #[test]
    fn test_hysteria2_link_auth_matches_generated_inbound() {
        // The link password and the inbound auth must be the same value,
        // otherwise the client cannot authenticate.
        let auth = "SharedAuthValue0123456789abcdefg";
        let inbound = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            auth,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
        );
        let link = ConfigManager::generate_hysteria2_client_link(
            auth,
            "203.0.113.1",
            11451,
            "www.bing.com",
            "e",
            IpVersion::IPv4,
            "AA",
        );
        assert_eq!(inbound["settings"]["users"][0]["auth"], auth);
        assert!(link.contains(&format!("hysteria2://{auth}@")));
    }

    #[test]
    fn test_hysteria2_batch_signature_is_constructible() {
        // Compile-time check: the batch entry point has the shape the handler
        // will call. `let _ = ...` avoids invoking the async body.
        let _f: fn(usize, IpVersion) -> _ =
            |count, ip_version| ConfigManager::batch_create_hysteria2_xray(count, ip_version);
    }

    #[test]
    fn test_reused_password_generator_is_32_alphanumeric() {
        use crate::core::singbox::hysteria2::Hysteria2Config;
        let pw = Hysteria2Config::generate_password();
        assert_eq!(pw.len(), 32);
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
