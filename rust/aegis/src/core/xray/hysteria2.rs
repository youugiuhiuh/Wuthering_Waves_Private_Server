use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::types::IpVersion;

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
}
