use crate::core::paths;
use crate::core::subscription::server::proto::ProxyConfig;
use std::fs;
use std::path::Path;

fn generate_config_id(protocol: &str, host: &str, port: u32) -> String {
    format!("{}-{}-{}", protocol, host, port)
}

fn scan_xray_configs() -> Vec<ProxyConfig> {
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
            let host = match inbound.get("host").and_then(|v| v.as_str()) {
                Some(h) => h.to_string(),
                None => continue,
            };
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
            let reality_settings = stream_settings.and_then(|s| s.get("realitySettings"));
            let sni = reality_settings
                .and_then(|r| r.get("serverName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pin_sha256 = reality_settings
                .and_then(|r| r.get("fingerprint"))
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
                pin_sha256,
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
                alpn: String::new(),
                congestion_control: String::new(),
                cert_sha256: String::new(),
            });
        }
    }
    configs
}

fn scan_singbox_configs() -> Vec<ProxyConfig> {
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
        let outbounds = match json.get("outbounds").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for outbound in outbounds {
            let protocol = match outbound.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };
            match protocol {
                "hysteria2" | "hy2" => {
                    let port = outbound.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let host = match outbound.get("server").and_then(|v| v.as_str()) {
                        Some(h) => h.to_string(),
                        None => continue,
                    };
                    let password = outbound
                        .get("password")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let sni = outbound
                        .get("sni")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let obfs_type = outbound
                        .get("obfs")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let obfs_password = outbound
                        .get("obfs-password")
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
                        tag: String::new(),
                        obfs_type,
                        obfs_password,
                        hop_port_start: 0,
                        hop_port_end: 0,
                        alpn: String::new(),
                        congestion_control: String::new(),
                        cert_sha256: String::new(),
                    });
                }
                "tuic" => {
                    let port = outbound.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let host = match outbound.get("server").and_then(|v| v.as_str()) {
                        Some(h) => h.to_string(),
                        None => continue,
                    };
                    let password = outbound
                        .get("password")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let uuid = outbound
                        .get("uuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let alpn = outbound
                        .get("alpn")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let congestion_control = outbound
                        .get("congestion_control")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cert_sha256 = outbound
                        .get("cert_sha256")
                        .or_else(|| outbound.get("server_cert_sha256"))
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
                        sni: String::new(),
                        pin_sha256: String::new(),
                        public_key: String::new(),
                        short_id: String::new(),
                        transport: String::new(),
                        path: String::new(),
                        flow: String::new(),
                        tag: String::new(),
                        obfs_type: String::new(),
                        obfs_password: String::new(),
                        hop_port_start: 0,
                        hop_port_end: 0,
                        alpn,
                        congestion_control,
                        cert_sha256,
                    });
                }
                _ => {}
            }
        }
    }
    configs
}

pub fn aggregate_all() -> Vec<ProxyConfig> {
    let mut configs = scan_xray_configs();
    configs.extend(scan_singbox_configs());
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
        let configs = aggregate_all();
        assert!(configs.is_empty());
    }
}
