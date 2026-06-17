//! Integration test: KCP FinalMask JSON format validated against xray-core
//!
//! Uses xray-core's `convert pb` command to verify that KCP FinalMask JSON
//! output is accepted by xray-core v26.6.1+. Skips if `xray` is not installed.

use serde_json::json;
use std::process::Command;

fn xray_version() -> Option<String> {
    let output = Command::new("xray").arg("version").output().ok()?;
    if output.status.success() {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(|s| s.to_string())
    } else {
        None
    }
}

fn validate_config(config: &serde_json::Value, name: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join(format!("kcp_test_{name}.json"));
    let file = std::fs::File::create(&config_path).unwrap();
    serde_json::to_writer_pretty(file, config).unwrap();

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

fn build_inbound_config(masks: &[serde_json::Value]) -> serde_json::Value {
    json!({
        "inbounds": [{
            "listen": "127.0.0.1",
            "port": 9999,
            "protocol": "vless",
            "settings": {
                "clients": [{"id": "550e8400-e29b-41d4-a716-446655440000"}],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "kcp",
                "security": "none",
                "finalmask": { "udp": masks },
                "kcpSettings": {
                    "mtu": 1350,
                    "tti": 50,
                    "uplinkCapacity": 5,
                    "downlinkCapacity": 20
                }
            }
        }]
    })
}

#[test]
fn test_kcp_finalmask_xray_available() {
    let version = xray_version();
    assert!(
        version.is_some(),
        "xray binary not found — install xray-core v26.6.1+ to run these integration tests"
    );
    eprintln!("xray: {}", version.unwrap());
}

#[test]
fn test_kcp_finalmask_mkcp_legacy_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let tests: Vec<(&str, KcpMask)> = vec![
        (
            "header+dns+value",
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: Some("example.com".into()),
            },
        ),
        (
            "header+wechat+value",
            KcpMask::MkcpLegacy {
                header: Some("wechat".into()),
                value: Some("123456".into()),
            },
        ),
        (
            "value-only",
            KcpMask::MkcpLegacy {
                header: None,
                value: Some("pwd".into()),
            },
        ),
        (
            "no-settings",
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            },
        ),
    ];

    for (variant, mask) in tests {
        let config = build_inbound_config(&[mask.as_json()]);
        validate_config(&config, &format!("mkcp-legacy-{variant}"));
    }
}

#[test]
fn test_kcp_finalmask_noise_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let config = build_inbound_config(&[KcpMask::Noise.as_json()]);
    validate_config(&config, "noise");
}

#[test]
fn test_kcp_finalmask_salamander_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let tests: Vec<(&str, KcpMask)> = vec![
        (
            "with-packet-size",
            KcpMask::Salamander {
                password: "test".into(),
                packet_size: Some((512, 1200)),
            },
        ),
        (
            "no-packet-size",
            KcpMask::Salamander {
                password: "test".into(),
                packet_size: None,
            },
        ),
    ];

    for (variant, mask) in tests {
        let config = build_inbound_config(&[mask.as_json()]);
        validate_config(&config, &format!("salamander-{variant}"));
    }
}

#[test]
fn test_kcp_finalmask_sudoku_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let mask = KcpMask::Sudoku {
        password: "test".into(),
    };
    let config = build_inbound_config(&[mask.as_json()]);
    validate_config(&config, "sudoku");
}

#[test]
fn test_kcp_finalmask_xdns_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let mask = KcpMask::Xdns {
        domains: vec!["example.com".into()],
        resolvers: vec!["example.com+udp://8.8.8.8:53".into()],
    };
    let config = build_inbound_config(&[mask.as_json()]);
    validate_config(&config, "xdns");
}

#[test]
fn test_kcp_finalmask_xicmp_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let tests: Vec<(&str, KcpMask)> = vec![
        (
            "dgram+ips",
            KcpMask::Xicmp {
                dgram: true,
                ips: vec!["1.2.3.4".into(), "5.6.7.8".into()],
            },
        ),
        (
            "no-settings",
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![],
            },
        ),
    ];

    for (variant, mask) in tests {
        let config = build_inbound_config(&[mask.as_json()]);
        validate_config(&config, &format!("xicmp-{variant}"));
    }
}

#[test]
fn test_kcp_finalmask_realm_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let mask = KcpMask::Realm {
        url: "realm://token@example.com:443/id".into(),
        stun_servers: vec!["stun.l.google.com:19302".into()],
    };
    let config = build_inbound_config(&[mask.as_json()]);
    validate_config(&config, "realm");
}

#[test]
fn test_kcp_finalmask_combined_with_xray() {
    let _ = xray_version().expect("xray not available — skipping");
    use aegis::core::xray::kcp_mask::KcpMask;

    let masks: Vec<serde_json::Value> = vec![
        KcpMask::MkcpLegacy {
            header: Some("dns".into()),
            value: Some("example.com".into()),
        }
        .as_json(),
        KcpMask::Noise.as_json(),
        KcpMask::Salamander {
            password: "test".into(),
            packet_size: Some((512, 1200)),
        }
        .as_json(),
        KcpMask::Sudoku {
            password: "test".into(),
        }
        .as_json(),
        KcpMask::Xicmp {
            dgram: true,
            ips: vec!["1.2.3.4".into()],
        }
        .as_json(),
        KcpMask::Realm {
            url: "realm://token@example.com:443/id".into(),
            stun_servers: vec!["stun.l.google.com:19302".into()],
        }
        .as_json(),
    ];
    let config = build_inbound_config(&masks);
    validate_config(&config, "combined");
}
