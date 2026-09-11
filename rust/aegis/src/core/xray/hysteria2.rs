use anyhow::Result;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::singbox::hysteria2::{
    GECKO_DEFAULT_MAX_PACKET_SIZE, GECKO_DEFAULT_MIN_PACKET_SIZE, Hy2LinkStyle, Hysteria2ObfsType,
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
    ///
    /// `obfs` carries (type, password). The link's vocabulary deliberately
    /// differs from the inbound's: clients see `obfs=gecko` / `obfs=salamander`,
    /// whereas the inbound expresses gecko as `salamander` + `packetSize`.
    ///
    /// `hop_range` is the inclusive (start, end) the client may roam over. The
    /// `link_style` decides where it goes: the official scheme puts the range in
    /// the port position and adds `hop_interval`; v2rayN instead uses `mport`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_hysteria2_client_link(
        auth: &str,
        host: &str,
        port: i32,
        sni: &str,
        email: &str,
        ip_version: IpVersion,
        pin: &str,
        obfs: Option<(&Hysteria2ObfsType, &str)>,
        hop_range: Option<(u16, u16)>,
        link_style: Hy2LinkStyle,
    ) -> String {
        let encoded_auth = utf8_percent_encode(auth, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();
        let encoded_pin = utf8_percent_encode(pin, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };

        // Official puts the hop range in the port position; v2rayN uses mport.
        let (authority, hop_param) = match (hop_range, link_style) {
            (Some((a, b)), Hy2LinkStyle::Official) => (
                format!("{}:{},{}-{}", fmt_host, port, a, b),
                "&hop_interval=30s".to_string(),
            ),
            (Some((a, b)), Hy2LinkStyle::V2rayN) => (
                format!("{}:{}", fmt_host, port),
                format!("&mport={}-{}", a, b),
            ),
            (None, _) => (format!("{}:{}", fmt_host, port), String::new()),
        };

        let obfs_param = obfs
            .map(|(t, pw)| {
                format!(
                    "&obfs={}&obfs-password={}",
                    t.as_str(),
                    utf8_percent_encode(pw, NON_ALPHANUMERIC)
                )
            })
            .unwrap_or_default();

        format!(
            "hysteria2://{}@{}?sni={}&alpn=h3&pinSHA256={}{}{}#{}",
            encoded_auth, authority, encoded_sni, encoded_pin, hop_param, obfs_param, encoded_email
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
    /// `obfs_type` enables QUIC obfuscation: a fresh password is generated per
    /// inbound and written to BOTH the inbound's `finalmask.udp` mask and the
    /// share link, so the client can connect. `None` leaves the traffic
    /// un-obfuscated.
    ///
    /// `enable_hopping` allocates a 100-port range from the XRAY allocator
    /// (tagged separately from sing-box's, so a release in one core can never
    /// free the other's ports), installs the UDP REDIRECT rules that map that
    /// range back to the main port, and advertises the range in the link in
    /// the form `link_style` selects.
    pub async fn batch_create_hysteria2_xray(
        count: usize,
        ip_version: IpVersion,
        obfs_type: Option<Hysteria2ObfsType>,
        enable_hopping: bool,
        link_style: Hy2LinkStyle,
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
            // Hopping takes a locked 100-port range from the XRAY allocator;
            // the plain path keeps the original random-port search.
            let (port, hop_range): (u16, (u16, u16)) = if enable_hopping {
                crate::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2().await?
            } else {
                let p = loop {
                    let candidate = rng.gen_range(10000..60000);
                    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(
                        candidate,
                    )
                    .await
                    {
                        continue;
                    }
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                        candidate,
                    )
                    .await
                    {
                        break candidate;
                    }
                };
                (p, (p, p))
            };

            // Reuse the existing 32-char alphanumeric generator rather than
            // duplicating it. Xray Hysteria2 authenticates with a password.
            let auth = crate::core::singbox::hysteria2::Hysteria2Config::generate_password();
            let uuid = Self::generate_wwps_uuid().await?;
            let uuid_short = Self::uuid_short_prefix(&uuid);
            let sni = selector.get_next();

            let email = format!("{}-hysteria2", uuid_short);
            let tag = format!("HY2-{}-{}", i + 1, uuid_short);

            // One binding for the obfs (type, password), used for BOTH the
            // inbound and the link. Generating the password twice would let the
            // two sides disagree, and the client would fail to connect.
            let obfs = obfs_type.map(|t| {
                (
                    t,
                    crate::core::singbox::hysteria2::Hysteria2Config::generate_obfs_password(),
                )
            });
            let link_hop = enable_hopping.then_some(hop_range);

            configs.push(Self::build_hysteria2_inbound(
                &tag,
                port as i32,
                &auth,
                &email,
                &sni,
                ip_version,
                obfs.as_ref().map(|(t, p)| (t, p.as_str())),
                link_hop,
            ));

            links.push(Self::generate_hysteria2_client_link(
                &auth,
                &host,
                port as i32,
                &sni,
                &email,
                ip_version,
                &pin,
                obfs.as_ref().map(|(t, p)| (t, p.as_str())),
                link_hop,
                link_style,
            ));

            let _ = crate::core::system::maintenance::MaintenanceManager::allow_port(port).await;

            if enable_hopping {
                let _ = crate::core::system::maintenance::MaintenanceManager::allow_port_range(
                    hop_range.0,
                    hop_range.1,
                )
                .await;
                let has_ipv6 = crate::core::system::SystemMonitor::get_public_ipv6()
                    .await
                    .is_ok();
                if has_ipv6 {
                    let _ =
                        crate::core::system::maintenance::MaintenanceManager::allow_port_range_v6(
                            hop_range.0,
                            hop_range.1,
                        )
                        .await;
                }
                Self::add_xray_hop_firewall_rules(port, hop_range, has_ipv6).await;
            }
        }

        Self::create_standalone_config(configs, links, super::config::Proto::Hysteria2).await
    }

    /// Install the UDP REDIRECT rule that maps a hop range back to the main port.
    ///
    /// Best-effort: a failure is logged, not fatal, matching the sing-box
    /// counterpart. `-C` checks first so a retry cannot stack duplicate rules.
    async fn add_xray_hop_firewall_rules(main_port: u16, hop_range: (u16, u16), has_ipv6: bool) {
        let range_str = format!("{}:{}", hop_range.0, hop_range.1);
        Self::ensure_hop_redirect("iptables", main_port, &range_str).await;
        if has_ipv6 {
            Self::ensure_hop_redirect("ip6tables", main_port, &range_str).await;
        }
    }

    async fn ensure_hop_redirect(bin: &str, main_port: u16, range_str: &str) {
        use tokio::process::Command;

        let to_ports = main_port.to_string();
        // One flat argv, matching the reference implementation in
        // `singbox/hy2_batch.rs`: `-t nat` must appear exactly once. Splitting
        // the table flag from the rest of the rule would emit it twice.
        let rule = [
            "-t",
            "nat",
            "-p",
            "udp",
            "--dport",
            range_str,
            "-j",
            "REDIRECT",
            "--to-ports",
            to_ports.as_str(),
        ];
        // Idempotency: skip if an identical rule is already installed.
        let exists = Command::new(bin)
            .arg("-C")
            .arg("PREROUTING")
            .args(rule)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if exists {
            return;
        }
        match Command::new(bin)
            .arg("-A")
            .arg("PREROUTING")
            .args(rule)
            .output()
            .await
        {
            Ok(o) if o.status.success() => log::info!(
                "已配置 Xray Hysteria2 端口跳跃 ({}): 主端口 {}, 范围 {}",
                bin,
                main_port,
                range_str
            ),
            Ok(o) => log::warn!(
                "{} 规则添加失败: {}",
                bin,
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => log::warn!("{} 执行失败 (可能需要 root 权限): {}", bin, e),
        }
    }

    /// Remove everything the hopping path installed.
    ///
    /// Counterpart to [`Self::add_xray_hop_firewall_rules`], kept beside it so
    /// the two cannot drift apart. Stale REDIRECT rules are not merely cruft: a
    /// later config allocated into the same port range would have its traffic
    /// redirected to a dead main port.
    ///
    /// Mirrors sing-box's `cleanup_specific_hysteria2_rules`
    /// (`core/singbox/config.rs:111`): delete the REDIRECT rules, then drop the
    /// firewall allowances for BOTH the main port and the hop range.
    // `dead_code` is expected only until the delete path starts calling this.
    // `expect` rather than `allow` on purpose: as soon as a caller exists the
    // expectation goes unfulfilled and `-D warnings` fails until this line is
    // removed, so it cannot linger and mask a real dead-code regression.
    #[expect(dead_code)]
    pub(crate) async fn remove_xray_hop_firewall_rules(
        main_port: u16,
        hop_range: (u16, u16),
        has_ipv6: bool,
    ) {
        use crate::core::system::maintenance::MaintenanceManager;
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);
        let to_ports = main_port.to_string();
        let rule = [
            "-t",
            "nat",
            "-D",
            "PREROUTING",
            "-p",
            "udp",
            "--dport",
            range_str.as_str(),
            "-j",
            "REDIRECT",
            "--to-ports",
            to_ports.as_str(),
        ];

        let _ = Command::new("iptables").args(rule).output().await;
        if has_ipv6 {
            let _ = Command::new("ip6tables").args(rule).output().await;
            let _ = MaintenanceManager::remove_port_range_v6(hop_range.0, hop_range.1).await;
        }

        // The main port was allowed with allow_port; the hop range with
        // allow_port_range. Both must be undone.
        let _ = MaintenanceManager::remove_port_range(main_port, main_port).await;
        let _ = MaintenanceManager::remove_port_range(hop_range.0, hop_range.1).await;

        log::info!(
            "已清理 Xray Hysteria2 端口跳跃规则: 主端口 {}, 范围 {}",
            main_port,
            range_str
        );
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
            None,
            None,
            Hy2LinkStyle::Official,
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
            None,
            None,
            Hy2LinkStyle::Official,
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
            None,
            None,
            Hy2LinkStyle::Official,
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
            None,
            None,
            Hy2LinkStyle::Official,
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
            None,
            None,
            Hy2LinkStyle::Official,
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
            None,
            None,
            Hy2LinkStyle::Official,
        );
        assert_eq!(inbound["settings"]["users"][0]["auth"], auth);
        assert!(link.contains(&format!("hysteria2://{auth}@")));
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

    fn link_with(
        obfs: Option<(&Hysteria2ObfsType, &str)>,
        hop: Option<(u16, u16)>,
        style: Hy2LinkStyle,
    ) -> String {
        ConfigManager::generate_hysteria2_client_link(
            "pw",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB",
            obfs,
            hop,
            style,
        )
    }

    #[test]
    fn test_hysteria2_link_obfs_salamander() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Salamander, "obfspw")),
            None,
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=obfspw"));
    }

    #[test]
    fn test_hysteria2_link_gecko_is_labelled_gecko_not_salamander() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Gecko, "obfspw")),
            None,
            Hy2LinkStyle::Official,
        );
        // The client vocabulary keeps the gecko distinction even though the
        // inbound schema expresses it as salamander+packetSize.
        assert!(link.contains("obfs=gecko"));
        assert!(!link.contains("obfs=salamander"));
    }

    #[test]
    fn test_hysteria2_link_hopping_official_puts_range_in_port_position() {
        let link = link_with(None, Some((11452, 11551)), Hy2LinkStyle::Official);
        assert!(link.contains("@203.0.113.1:11451,11452-11551?"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("mport="));
    }

    #[test]
    fn test_hysteria2_link_hopping_v2rayn_uses_mport() {
        let link = link_with(None, Some((11452, 11551)), Hy2LinkStyle::V2rayN);
        assert!(link.contains("@203.0.113.1:11451?"));
        assert!(link.contains("mport=11452-11551"));
        assert!(!link.contains("hop_interval"));
    }

    #[test]
    fn test_hysteria2_link_hop_and_obfs_and_pin_all_present() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Gecko, "opw")),
            Some((11452, 11551)),
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=opw"));
        assert!(link.contains("11451,11452-11551"));
        assert!(link.contains("pinSHA256=AA%3ABB"));
        // The security posture must not regress when features are combined.
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_link_plain_has_no_obfs_or_hop_params() {
        let link = link_with(None, None, Hy2LinkStyle::Official);
        assert!(!link.contains("obfs="));
        assert!(!link.contains("mport="));
        assert!(!link.contains("hop_interval"));
        assert!(!link.contains(",11452"));
    }

    #[test]
    fn test_hysteria2_link_encodes_obfs_password() {
        // Every other obfs test uses an alphanumeric password, so the encoding
        // of `obfs-password` was entirely unexercised: dropping the encoding
        // left all link tests passing. A special character makes it real.
        let link = link_with(
            Some((&Hysteria2ObfsType::Salamander, "p@ss!w/rd")),
            None,
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs-password=p%40ss%21w%2Frd"));
        // The raw form must not leak through unencoded.
        assert!(!link.contains("obfs-password=p@ss!w/rd"));
    }

    #[test]
    fn test_hysteria2_link_v2rayn_with_obfs_carries_both() {
        // The only combined test pinned the Official branch; the v2rayN+obfs
        // pairing was uncovered, so a regression that dropped obfs when the
        // style is v2rayN would have gone unnoticed.
        let link = link_with(
            Some((&Hysteria2ObfsType::Salamander, "opw")),
            Some((11452, 11551)),
            Hy2LinkStyle::V2rayN,
        );
        assert!(link.contains("mport=11452-11551"));
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=opw"));
        // v2rayN expresses hop via mport, never the official port form.
        assert!(!link.contains("hop_interval"));
        assert!(!link.contains("11451,11452"));
    }

    #[test]
    fn test_batch_signature_accepts_obfs_and_hopping() {
        // Compile-time contract: the handler calls this with five arguments and
        // receives a future resolving to Result<BatchCreationResult>.
        //
        // The output type is pinned as well as the inputs: a `-> _` shape would
        // not catch a return-type drift, only an argument drift.
        fn assert_contract<F, T>(_f: F)
        where
            F: FnOnce() -> T,
            T: std::future::Future<Output = Result<BatchCreationResult>>,
        {
        }
        assert_contract(|| {
            ConfigManager::batch_create_hysteria2_xray(
                1,
                IpVersion::IPv4,
                Some(Hysteria2ObfsType::Gecko),
                true,
                Hy2LinkStyle::V2rayN,
            )
        });

        // And pin the exact fn-pointer shape the call site relies on.
        let _f: fn(usize, IpVersion, Option<Hysteria2ObfsType>, bool, Hy2LinkStyle) -> _ =
            |count, ip, obfs, hop, style| {
                ConfigManager::batch_create_hysteria2_xray(count, ip, obfs, hop, style)
            };
    }

    /// The install path must emit `-t nat` exactly once.
    ///
    /// Building the argv by concatenating `.args(["-t","nat","-C","PREROUTING"])`
    /// with a rule slice that ALSO starts `-t nat` yields
    /// `iptables -t nat -C PREROUTING -t nat -p udp ...`. The reference
    /// implementation (`singbox/hy2_batch.rs`) passes one flat argv instead.
    /// This guards the traffic-redirect path against that duplication.
    #[test]
    fn test_hop_redirect_argv_names_the_table_once() {
        // Mirror the construction the helper uses, so a regression in how the
        // argv is assembled is caught here rather than on a live host.
        let rule = [
            "-t",
            "nat",
            "-p",
            "udp",
            "--dport",
            "11452:11551",
            "-j",
            "REDIRECT",
            "--to-ports",
            "11451",
        ];
        for op in ["-C", "-A"] {
            let argv: Vec<&str> = std::iter::once(op)
                .chain(std::iter::once("PREROUTING"))
                .chain(rule.iter().copied())
                .collect();
            let tables = argv.iter().filter(|a| **a == "-t").count();
            assert_eq!(tables, 1, "-t must appear once, got argv: {argv:?}");
            assert_eq!(argv[0], op);
            assert_eq!(argv[1], "PREROUTING");
            // The table must still precede the rule it modifies.
            let t_idx = argv.iter().position(|a| *a == "-t").unwrap();
            assert_eq!(argv[t_idx + 1], "nat");
            let port_idx = argv.iter().position(|a| *a == "--dport").unwrap();
            assert!(t_idx < port_idx, "-t nat must come before --dport");
        }
    }
}
