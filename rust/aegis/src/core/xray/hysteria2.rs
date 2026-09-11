use anyhow::Result;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::singbox::hysteria2::{
    GECKO_DEFAULT_MAX_PACKET_SIZE, GECKO_DEFAULT_MIN_PACKET_SIZE, Hysteria2ObfsType,
};
use crate::core::types::{BatchCreationResult, IpVersion};

impl ConfigManager {
    /// Build a single Xray `hysteria` inbound (Hysteria2).
    ///
    /// Xray splits Hysteria2 configuration across two layers and BOTH must be
    /// present: the protocol layer (`protocol: "hysteria"` + `settings.users`)
    /// and the transport layer (`network: "hysteria"` + `hysteriaSettings`).
    /// Either alone produces an inbound that never authenticates.
    ///
    /// `obfs` carries (type, password). Xray has no separate gecko type, so a
    /// gecko request is written as `salamander` plus a `packetSize`; plain
    /// salamander omits `packetSize`. The password must match the client's.
    ///
    /// `hop_range` is the inclusive (start, end) of the ports the client may
    /// roam over; the inbound advertises it via `finalmask.quicParams.udpHop`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_hysteria2_inbound(
        tag: &str,
        port: i32,
        auth: &str,
        email: &str,
        sni: &str,
        ip_version: IpVersion,
        obfs: Option<(&Hysteria2ObfsType, &str)>,
        hop_range: Option<(u16, u16)>,
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => "0.0.0.0",
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => "::",
        };

        let mut inbound = json!({
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
        });

        // Both features live under one `finalmask` object. Build it once and
        // insert once: assigning `finalmask` per-feature would replace the
        // whole object and silently drop the earlier feature.
        let mut finalmask = serde_json::Map::new();
        if let Some((obfs_type, obfs_password)) = obfs {
            finalmask.insert(
                "udp".to_string(),
                json!([Self::build_hysteria2_udp_mask(obfs_type, obfs_password)]),
            );
        }
        if let Some((hop_start, hop_end)) = hop_range {
            finalmask.insert(
                "quicParams".to_string(),
                json!({
                    "udpHop": {
                        "ports": format!("{}-{}", hop_start, hop_end),
                        "interval": 30
                    }
                }),
            );
        }
        if !finalmask.is_empty() {
            inbound["streamSettings"]["finalmask"] = Value::Object(finalmask);
        }

        inbound
    }

    /// Build one UDPMask entry for Hysteria2 obfuscation.
    ///
    /// Xray exposes only the `salamander` type; gecko is chosen by supplying
    /// `packetSize`, which enables extra fragmentation/padding of QUIC
    /// long-header packets. Upper bound must not exceed 2048 per the Xray docs.
    pub(crate) fn build_hysteria2_udp_mask(obfs_type: &Hysteria2ObfsType, password: &str) -> Value {
        let mut settings = serde_json::Map::new();
        settings.insert("password".to_string(), json!(password));
        if *obfs_type == Hysteria2ObfsType::Gecko {
            settings.insert(
                "packetSize".to_string(),
                json!(format!(
                    "{}-{}",
                    GECKO_DEFAULT_MIN_PACKET_SIZE, GECKO_DEFAULT_MAX_PACKET_SIZE
                )),
            );
        }
        json!({
            "type": "salamander",
            "settings": settings
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

    /// Number of batch files a `count`-inbound batch will write: 1 for any
    /// non-empty batch, 0 for an empty one. Each batch call serializes all of
    /// its inbounds into a single `batch_xray_hysteria2_*_inbounds.json` file,
    /// so the file delta is NOT `count` (which would mix inbounds with files).
    fn batch_file_delta(count: usize) -> usize {
        usize::from(count > 0)
    }

    /// Enforce the Xray Hysteria2 config cap.
    ///
    /// Both arguments are FILE counts: `existing_files` is the number of
    /// `batch_xray_hysteria2_*_inbounds.json` files already present, and
    /// `requested_files` is how many files the current call will create
    /// (1 for a non-empty batch, 0 for an empty one). The cap is on batch
    /// files, matching the original sing-box `check_hysteria2_limit` which
    /// also counts files — one batch writes exactly ONE file regardless of how
    /// many inbounds it holds, so counting inbounds would mix units.
    ///
    /// Uses `saturating_add` so an absurd `requested_files` (callback data is
    /// attacker-influenced) cannot wrap into a small number that passes.
    fn check_xray_hysteria2_capacity(existing_files: usize, requested_files: usize) -> Result<()> {
        const MAX_XRAY_HY2_CONFIGS: usize = 50;
        if existing_files.saturating_add(requested_files) > MAX_XRAY_HY2_CONFIGS {
            return Err(anyhow::anyhow!(
                "已达到最大 Xray Hysteria2 配置文件数量限制（{}个），当前 {} 个配置文件",
                MAX_XRAY_HY2_CONFIGS,
                existing_files
            ));
        }
        Ok(())
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

        // The shared PortAllocator::check_hysteria2_limit() counts sing-box
        // configs (it scans /etc/wwps/wwps-box/conf for the substring
        // "hysteria2"), so it cannot see Xray configs, which live in
        // xray::CONF_DIR and contain `"protocol": "hysteria"`. Count this
        // core's own configs instead, via the same prefix used for filtering.
        let existing_files = Self::list_inbound_files_by_proto(super::config::Proto::Hysteria2)
            .await
            .map(|f| f.len())
            .unwrap_or(0);
        // The cap counts FILES in this core's conf dir (matching the original
        // sing-box `check_hysteria2_limit`, which also counts files). Each
        // batch call writes exactly ONE file holding `count` inbounds, so the
        // file delta is 1 -- not `count`, which would mix units (inbounds vs
        // files) and both under-count existing usage and over-count the
        // incoming request.
        let requested_files = Self::batch_file_delta(count);
        Self::check_xray_hysteria2_capacity(existing_files, requested_files)?;

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
                &tag, port, &auth, &email, &sni, ip_version, None, None,
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
            None,
            None,
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
    fn test_hysteria2_obfs_salamander_writes_finalmask_udp() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Salamander, "obfspw")),
            None,
        );
        let udp = &cfg["streamSettings"]["finalmask"]["udp"][0];
        assert_eq!(udp["type"], "salamander");
        assert_eq!(udp["settings"]["password"], "obfspw");
        // Plain salamander must NOT carry packetSize; that is what makes it
        // gecko. A stray packetSize here would silently enable gecko.
        assert!(udp["settings"].get("packetSize").is_none());
    }

    #[test]
    fn test_hysteria2_obfs_gecko_is_salamander_plus_packetsize() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Gecko, "obfspw")),
            None,
        );
        let udp = &cfg["streamSettings"]["finalmask"]["udp"][0];
        // Xray has no "gecko" type: gecko IS salamander with a packetSize.
        assert_eq!(udp["type"], "salamander");
        assert_ne!(udp["type"], "gecko");
        assert_eq!(udp["settings"]["password"], "obfspw");
        assert_eq!(
            udp["settings"]["packetSize"],
            format!(
                "{}-{}",
                GECKO_DEFAULT_MIN_PACKET_SIZE, GECKO_DEFAULT_MAX_PACKET_SIZE
            )
        );
    }

    #[test]
    fn test_hysteria2_no_obfs_omits_finalmask_entirely() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            None,
        );
        // Absent, not an empty object: an empty finalmask is not the same
        // config to Xray and would be a schema surprise.
        assert!(cfg["streamSettings"].get("finalmask").is_none());
    }

    #[test]
    fn test_hysteria2_hop_writes_quicparams_udphop() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            Some((11452, 11551)),
        );
        let hop = &cfg["streamSettings"]["finalmask"]["quicParams"]["udpHop"];
        assert_eq!(hop["ports"], "11452-11551");
        assert_eq!(hop["interval"], 30);
    }

    #[test]
    fn test_hysteria2_hop_and_obfs_coexist_in_one_finalmask() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Gecko, "pw")),
            Some((11452, 11551)),
        );
        let fm = &cfg["streamSettings"]["finalmask"];
        // Both keys must live under the SAME finalmask object. Writing either
        // one by replacing `finalmask` wholesale would drop the other.
        assert_eq!(fm["udp"][0]["type"], "salamander");
        assert_eq!(fm["quicParams"]["udpHop"]["ports"], "11452-11551");
    }

    #[test]
    fn test_hysteria2_both_features_off_omits_finalmask() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            None,
        );
        assert!(cfg["streamSettings"].get("finalmask").is_none());
    }

    #[test]
    fn test_hysteria2_obfs_only_adds_no_quicparams() {
        // Hop is off, so `udpHop` must not appear at all. Without this, a bug
        // that emits `quicParams` whenever obfs is on would slip through: the
        // both-off test passes `None` for hop too and cannot see it.
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Salamander, "pw")),
            None,
        );
        let fm = &cfg["streamSettings"]["finalmask"];
        assert_eq!(fm["udp"][0]["type"], "salamander");
        assert!(
            fm.get("quicParams").is_none(),
            "obfs-only config must not carry quicParams: {fm}"
        );
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
            None,
            None,
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

    #[test]
    fn test_batch_file_delta_maps_inbounds_to_files() {
        // A batch writes exactly ONE file regardless of how many inbounds it
        // holds. This is the regression guard: passing `count` (inbounds)
        // instead of this delta is the exact bug that mixed units.
        assert_eq!(ConfigManager::batch_file_delta(50), 1);
        assert_eq!(ConfigManager::batch_file_delta(1), 1);
        assert_eq!(ConfigManager::batch_file_delta(0), 0);
    }

    #[test]
    fn test_xray_hysteria2_capacity_enforces_cap_at_boundary() {
        // Truth table in FILE units: the cap is 50 batch files.
        assert!(ConfigManager::check_xray_hysteria2_capacity(0, 1).is_ok()); // first file
        assert!(ConfigManager::check_xray_hysteria2_capacity(49, 1).is_ok()); // 49 -> 50 (reaches cap)
        assert!(ConfigManager::check_xray_hysteria2_capacity(50, 1).is_err()); // 50 -> 51 (over cap)
    }

    #[test]
    fn test_first_batch_of_50_inbounds_allowed_then_50th_file_rejected() {
        // Clean server + a 50-inbound batch = 1 file: allowed.
        // Asserted in FILE units via an explicit literal, not through
        // `batch_file_delta`, so this test still fails if the helper is ever
        // reverted to return `count` (a `(0, 50)` case would still be allowed
        // and would hide the regression).
        assert_eq!(ConfigManager::batch_file_delta(50), 1);
        assert!(ConfigManager::check_xray_hysteria2_capacity(0, 1).is_ok());
        // Once 50 files exist, the next single-file batch is rejected.
        assert!(ConfigManager::check_xray_hysteria2_capacity(50, 1).is_err());
    }

    #[test]
    fn test_xray_hysteria2_capacity_error_names_cap_and_current_count() {
        let err = ConfigManager::check_xray_hysteria2_capacity(50, 1)
            .expect_err("50 existing files + 1 more must exceed the cap");
        let msg = err.to_string();
        assert!(msg.contains("50"), "error must name the cap: {msg}");
        assert!(
            msg.contains("配置文件"),
            "error must name the unit (config files): {msg}"
        );
        assert!(
            msg.contains("50 个配置文件"),
            "error must report the existing file count: {msg}"
        );
    }

    #[test]
    fn test_xray_hysteria2_capacity_rejects_without_wrapping() {
        // Defensive: even though the call site only ever passes 0 or 1, a huge
        // value must be rejected, not wrapped below the cap.
        assert!(ConfigManager::check_xray_hysteria2_capacity(0, usize::MAX).is_err());
        assert!(ConfigManager::check_xray_hysteria2_capacity(10, usize::MAX - 5).is_err());
    }

    /// The built inbound must match the structure documented by Xray for a
    /// `hysteria` inbound, and survive a JSON round trip (it is written to
    /// disk as JSON and read back by Xray-core).
    ///
    /// Reference: https://github.com/XTLS/Xray-examples (Hysteria2/server.jsonc)
    /// and https://xtls.github.io/config/inbounds/hysteria.html
    #[test]
    fn test_hysteria2_inbound_matches_xray_reference_structure_and_round_trips() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "IN-Hysteria2",
            11451,
            AUTH,
            "abcd1234-hysteria2",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            None,
        );

        // Round trip: this is what actually happens on disk. If a non-string
        // key or non-serialisable value ever leaked in, this fails.
        let serialized = serde_json::to_string(&cfg).expect("inbound must serialize");
        let back: Value = serde_json::from_str(&serialized).expect("inbound must deserialize");
        assert_eq!(back, cfg, "inbound must survive a JSON round trip");

        // Exactly the top-level keys the reference inbound uses (plus tag and
        // sniffing, which the reference config sets globally instead).
        let obj = cfg.as_object().expect("inbound must be a JSON object");
        for key in [
            "protocol",
            "port",
            "listen",
            "tag",
            "settings",
            "streamSettings",
        ] {
            assert!(obj.contains_key(key), "missing required key: {key}");
        }

        // The reference sets `settings.version` alongside the transport-level
        // version. Both layers must agree, or Xray rejects the config.
        assert_eq!(cfg["settings"]["version"], 2);
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["version"], 2);

        // NOTE: we additionally set `settings.users[].auth`, which the
        // reference config omits (it relies on transport-level `auth` alone).
        // Both are documented as valid; we set both so `auth` stays consistent
        // whichever layer a client keys off. Asserted deliberately, not
        // incidentally.
        assert_eq!(cfg["settings"]["users"][0]["auth"], AUTH);
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["auth"], AUTH);
    }
}
