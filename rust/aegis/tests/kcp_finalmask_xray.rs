//! Integration test: KCP FinalMask JSON format validated against xray-core
//!
//! Uses xray-core's `convert pb` command to verify that the JSON produced by
//! `ConfigManager::build_kcp_inbound()` (the production code path) is accepted
//! by xray-core v26.6.1+. Skips if `xray` is not installed.

use aegis::core::types::IpVersion;
use aegis::core::xray::config::ConfigManager;
use aegis::core::xray::kcp_mask::KcpMask;
use serde_json::json;
use std::process::Command;

fn xray_available() -> bool {
    Command::new("xray")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn validate_inbound(config: &serde_json::Value, name: &str) {
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
