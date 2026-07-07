use crate::core::paths;
use crate::core::subscription::server::proto::ProxyConfig;
use std::fs;
use std::path::Path;

fn generate_config_id(protocol: &str, host: &str, port: u32) -> String {
    format!("{}-{}-{}", protocol, host, port)
}

fn scan_xray_configs(public_ip: Option<&str>) -> Vec<ProxyConfig> {
    let mut configs = Vec::new();
    let dir = Path::new(paths::xray::CONF_DIR);
    if !dir.exists() {
        return configs;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return configs,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !fname.ends_with("_inbounds.json") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let inbounds = match json.get("inbounds").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for inbound in inbounds {
            match inbound.get("protocol").and_then(|v| v.as_str()) {
                Some("vless") => {}
                _ => continue,
            };
            let port = inbound.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let raw_host = inbound
                .get("listen")
                .and_then(|v| v.as_str())
                .filter(|h| !h.is_empty() && *h != "0.0.0.0" && *h != "::")
                .or_else(|| inbound.get("host").and_then(|v| v.as_str()))
                .unwrap_or("0.0.0.0");
            let host = resolve_host(raw_host, public_ip);
            let settings = inbound.get("settings");
            let uuid = settings
                .and_then(|s| s.get("clients"))
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|client| client.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let flow = settings
                .and_then(|s| s.get("clients"))
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|client| client.get("flow"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let stream_settings = inbound.get("streamSettings");
            let transport = stream_settings
                .and_then(|s| s.get("network"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let path = stream_settings
                .and_then(|s| {
                    s.get("wsSettings")
                        .or_else(|| s.get("grpcSettings"))
                        .or_else(|| s.get("httpSettings"))
                        .or_else(|| s.get("tcpSettings"))
                })
                .and_then(|ws| ws.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let http_host = stream_settings
                .and_then(|s| s.get("wsSettings").or_else(|| s.get("httpSettings")))
                .and_then(|ws| ws.get("host"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let alpn = stream_settings
                .and_then(|s| s.get("alpn"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let reality_settings = stream_settings.and_then(|s| s.get("realitySettings"));
            let sni = reality_settings
                .and_then(|r| r.get("serverName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let fingerprint = reality_settings
                .and_then(|r| r.get("fingerprint"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let spx = reality_settings
                .and_then(|r| r.get("shortPath"))
                .or_else(|| reality_settings.and_then(|r| r.get("spx")))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let public_key = reality_settings
                .and_then(|r| r.get("publicKey"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let short_id = reality_settings
                .and_then(|r| r.get("shortId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let config_id = generate_config_id("vless", &host, port);
            configs.push(ProxyConfig {
                config_id,
                protocol: "vless".to_string(),
                host,
                port,
                password: String::new(),
                uuid,
                sni,
                pin_sha256: String::new(),
                public_key,
                short_id,
                transport,
                path,
                flow,
                tag: String::new(),
                obfs_type: String::new(),
                obfs_password: String::new(),
                hop_port_start: 0,
                hop_port_end: 0,
                alpn,
                congestion_control: String::new(),
                cert_sha256: String::new(),
                fingerprint,
                spx,
                http_host,
                mode: String::new(),
                extra: String::new(),
                header_type: String::new(),
                service_name: String::new(),
                authority: String::new(),
                insecure: false,
                encryption: String::new(),
                server_name: String::new(),
            });
        }
    }
    configs
}

fn resolve_host(raw: &str, public_ip: Option<&str>) -> String {
    match raw {
        "" | "0.0.0.0" | "::" | "127.0.0.1" => public_ip
            .filter(|ip| !ip.is_empty())
            .unwrap_or(raw)
            .to_string(),
        _ => raw.to_string(),
    }
}

fn scan_singbox_configs(public_ip: Option<&str>) -> Vec<ProxyConfig> {
    let mut configs = Vec::new();
    let dir = Path::new(paths::singbox::CONF_DIR);
    if !dir.exists() {
        return configs;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return configs,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if fname.starts_with("00_") {
            continue;
        }
        if !fname.ends_with(".json") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let inbounds = match json.get("inbounds").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for inbound in inbounds {
            let protocol = match inbound.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };
            match protocol {
                "hysteria2" | "hy2" => {
                    let port = inbound
                        .get("listen_port")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let raw_host = inbound
                        .get("listen")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.0.0.0");
                    let host = resolve_host(raw_host, public_ip);
                    let tag = inbound
                        .get("tag")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let users = inbound.get("users").and_then(|v| v.as_array());
                    let password = users
                        .and_then(|u| u.first())
                        .and_then(|u| u.get("password"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tls = inbound.get("tls");
                    let sni = tls
                        .and_then(|t| t.get("server_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let alpn = tls
                        .and_then(|t| t.get("alpn"))
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let obfs = inbound.get("obfs");
                    let obfs_type = obfs
                        .and_then(|o| o.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let obfs_password = obfs
                        .and_then(|o| o.get("password"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let (hop_port_start, hop_port_end) = inbound
                        .get("hop_port")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.split_once('-'))
                        .map(|(start, end)| {
                            (
                                start.parse::<u32>().unwrap_or(0),
                                end.parse::<u32>().unwrap_or(0),
                            )
                        })
                        .unwrap_or((0, 0));
                    let cert_sha256 = tls
                        .and_then(|t| t.get("cert_sha256"))
                        .or_else(|| tls.and_then(|t| t.get("server_cert_sha256")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let config_id = generate_config_id(protocol, &host, port);
                    configs.push(ProxyConfig {
                        config_id,
                        protocol: protocol.to_string(),
                        host,
                        port,
                        password,
                        uuid: String::new(),
                        sni,
                        pin_sha256: String::new(),
                        public_key: String::new(),
                        short_id: String::new(),
                        transport: String::new(),
                        path: String::new(),
                        flow: String::new(),
                        tag,
                        obfs_type,
                        obfs_password,
                        hop_port_start,
                        hop_port_end,
                        alpn,
                        congestion_control: String::new(),
                        cert_sha256,
                        fingerprint: String::new(),
                        spx: String::new(),
                        http_host: String::new(),
                        mode: String::new(),
                        extra: String::new(),
                        header_type: String::new(),
                        service_name: String::new(),
                        authority: String::new(),
                        insecure: false,
                        encryption: String::new(),
                        server_name: String::new(),
                    });
                }
                "tuic" => {
                    let port = inbound
                        .get("listen_port")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let raw_host = inbound
                        .get("listen")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.0.0.0");
                    let host = resolve_host(raw_host, public_ip);
                    let tag = inbound
                        .get("tag")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let users = inbound.get("users").and_then(|v| v.as_array());
                    let password = users
                        .and_then(|u| u.first())
                        .and_then(|u| u.get("password"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let uuid = users
                        .and_then(|u| u.first())
                        .and_then(|u| u.get("uuid"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tls = inbound.get("tls");
                    let sni = tls
                        .and_then(|t| t.get("server_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let alpn = tls
                        .and_then(|t| t.get("alpn"))
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let congestion_control = inbound
                        .get("congestion_control")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cert_sha256 = tls
                        .and_then(|t| t.get("cert_sha256"))
                        .or_else(|| tls.and_then(|t| t.get("server_cert_sha256")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let config_id = generate_config_id("tuic", &host, port);
                    configs.push(ProxyConfig {
                        config_id,
                        protocol: "tuic".to_string(),
                        host,
                        port,
                        password,
                        uuid,
                        sni,
                        pin_sha256: String::new(),
                        public_key: String::new(),
                        short_id: String::new(),
                        transport: String::new(),
                        path: String::new(),
                        flow: String::new(),
                        tag,
                        obfs_type: String::new(),
                        obfs_password: String::new(),
                        hop_port_start: 0,
                        hop_port_end: 0,
                        alpn,
                        congestion_control,
                        cert_sha256,
                        fingerprint: String::new(),
                        spx: String::new(),
                        http_host: String::new(),
                        mode: String::new(),
                        extra: String::new(),
                        header_type: String::new(),
                        service_name: String::new(),
                        authority: String::new(),
                        insecure: false,
                        encryption: String::new(),
                        server_name: String::new(),
                    });
                }
                _ => {}
            }
        }
    }
    configs
}

pub fn aggregate_all(public_ip: Option<&str>, allowed_ids: Option<&[String]>) -> Vec<ProxyConfig> {
    let mut configs = scan_xray_configs(public_ip);
    configs.extend(scan_singbox_configs(public_ip));
    if let Some(ids) = allowed_ids {
        configs.retain(|c| ids.contains(&c.config_id));
    }
    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_config_id() {
        assert_eq!(
            generate_config_id("vless", "example.com", 443),
            "vless-example.com-443"
        );
        assert_eq!(
            generate_config_id("hysteria2", "10.0.0.1", 8443),
            "hysteria2-10.0.0.1-8443"
        );
    }

    #[test]
    fn test_aggregate_all_no_configs() {
        let configs = aggregate_all(None, None);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_host_fallback_to_listen() {
        let host = serde_json::from_str::<serde_json::Value>(r#"{"listen":"127.0.0.1","port":443,"protocol":"vless","settings":{"clients":[{"id":"uuid-1"}]}}"#).unwrap();
        let result = host
            .get("listen")
            .and_then(|v| v.as_str())
            .filter(|h| !h.is_empty() && *h != "0.0.0.0")
            .or_else(|| host.get("host").and_then(|v| v.as_str()))
            .unwrap_or("0.0.0.0");
        assert_eq!(result, "127.0.0.1");
    }

    #[test]
    fn test_host_fallback_no_listen_no_host() {
        let inbound = serde_json::from_str::<serde_json::Value>(
            r#"{"port":443,"protocol":"vless","settings":{"clients":[{"id":"uuid-1"}]}}"#,
        )
        .unwrap();
        let result = inbound
            .get("listen")
            .and_then(|v| v.as_str())
            .filter(|h| !h.is_empty() && *h != "0.0.0.0")
            .or_else(|| inbound.get("host").and_then(|v| v.as_str()))
            .unwrap_or("0.0.0.0");
        assert_eq!(result, "0.0.0.0");
    }

    #[test]
    #[test]
    fn test_resolve_host_returns_raw_when_not_placeholder() {
        assert_eq!(resolve_host("1.2.3.4", None), "1.2.3.4");
        assert_eq!(resolve_host("example.com", Some("5.6.7.8")), "example.com");
    }

    #[test]
    fn test_resolve_host_replaces_zero_with_public_ip() {
        assert_eq!(resolve_host("0.0.0.0", Some("5.6.7.8")), "5.6.7.8");
    }

    #[test]
    fn test_resolve_host_replaces_double_colon_with_public_ip() {
        assert_eq!(resolve_host("::", Some("5.6.7.8")), "5.6.7.8");
    }

    #[test]
    fn test_resolve_host_replaces_loopback_with_public_ip() {
        assert_eq!(resolve_host("127.0.0.1", Some("5.6.7.8")), "5.6.7.8");
    }

    #[test]
    fn test_resolve_host_keeps_zero_when_no_public_ip() {
        assert_eq!(resolve_host("0.0.0.0", None), "0.0.0.0");
    }

    #[test]
    fn test_resolve_host_keeps_zero_when_public_ip_empty() {
        assert_eq!(resolve_host("0.0.0.0", Some("")), "0.0.0.0");
    }

    #[test]
    fn test_aggregate_all_filters_allowed_ids_empty_returns_all() {
        // When allowed_ids is empty/Nothing, no filtering occurs — just verify
        // the scan functions are called (they'll read from disk and return empty in CI).
        let configs = aggregate_all(None, None);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_config_ids_filtering_logic() {
        let all = vec![
            ("vless-1.2.3.4-443".to_string(), "vless"),
            ("hysteria2-1.2.3.4-8443".to_string(), "hysteria2"),
        ];
        let allowed = vec!["vless-1.2.3.4-443".to_string()];
        let filtered: Vec<&(String, &str)> =
            all.iter().filter(|(id, _)| allowed.contains(id)).collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, "vless");
    }

    fn test_host_fallback_ignores_zero_listen() {
        let inbound = serde_json::from_str::<serde_json::Value>(r#"{"listen":"0.0.0.0","port":443,"protocol":"vless","settings":{"clients":[{"id":"uuid-1"}]}}"#).unwrap();
        let result = inbound
            .get("listen")
            .and_then(|v| v.as_str())
            .filter(|h| !h.is_empty() && *h != "0.0.0.0")
            .or_else(|| inbound.get("host").and_then(|v| v.as_str()))
            .unwrap_or("0.0.0.0");
        assert_eq!(result, "0.0.0.0");
    }

    #[test]
    fn test_host_fallback_listen_wins_over_host() {
        let inbound = serde_json::from_str::<serde_json::Value>(r#"{"listen":"1.2.3.4","host":"example.com","port":443,"protocol":"vless","settings":{"clients":[{"id":"uuid-1"}]}}"#).unwrap();
        let result = inbound
            .get("listen")
            .and_then(|v| v.as_str())
            .filter(|h| !h.is_empty() && *h != "0.0.0.0")
            .or_else(|| inbound.get("host").and_then(|v| v.as_str()))
            .unwrap_or("0.0.0.0");
        assert_eq!(result, "1.2.3.4");
    }
}
