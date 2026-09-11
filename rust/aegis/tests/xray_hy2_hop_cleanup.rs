//! End-to-end contract for Xray Hysteria2 hop-range cleanup on delete.
//!
//! The regression this locks down: if delete removes the config file but never
//! releases the locked hop range, the range stays in `.port_alloc` forever and
//! accumulates. sing-box had exactly this bug; the Xray path must not repeat it.

use aegis::core::xray::config::ConfigManager;
use aegis::core::xray::port_allocator::PortAllocator;
use serde_json::json;
use std::path::Path;

/// Main ports of ranges tagged for Xray Hysteria2 in an alloc file.
fn xray_main_ports(alloc: &Path) -> Vec<u16> {
    let Ok(content) = std::fs::read_to_string(alloc) else {
        return Vec::new();
    };
    let data: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    data["locked_ranges"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|r| r["protocol"].as_str() == Some("xray-hysteria2"))
                .filter_map(|r| r["start"].as_u64().map(|v| v as u16))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn delete_flow_releases_xray_hop_range_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");

    // Arrange: a created hopping config plus its locked range.
    let (main, _hop) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();
    assert_eq!(xray_main_ports(&alloc), vec![main]);

    let cfg_path = dir.path().join("batch_xray_hysteria2_it_inbounds.json");
    let cfg = json!({
        "inbounds": [{
            "protocol": "hysteria",
            "port": main,
            "streamSettings": {
                "finalmask": { "quicParams": { "udpHop": { "ports": format!("{}-{}", main + 1, main + 99) } } }
            }
        }]
    });
    std::fs::write(&cfg_path, cfg.to_string()).unwrap();

    // Act: the real delete path, pointed at this alloc file.
    ConfigManager::delete_specific_configuration_at(cfg_path.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    // Assert: range released AND file gone. A leak is the bug this test exists for.
    assert!(
        xray_main_ports(&alloc).is_empty(),
        "hop range {main} was not released on delete"
    );
    assert!(!cfg_path.exists(), "config file was not removed");
}

#[tokio::test]
async fn delete_of_a_non_hopping_config_leaves_ranges_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    // A live range that must survive deleting an unrelated plain config.
    let (keep, _) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();

    let cfg_path = dir.path().join("batch_xray_hysteria2_plain_inbounds.json");
    // The port is deliberately the SAME as the locked range's main port, so a
    // regression that drops the `udpHop` check (releasing any hysteria config's
    // port) would release this very range and fail — instead of releasing a
    // nonexistent port and silently passing.
    std::fs::write(
        &cfg_path,
        json!({"inbounds":[{"protocol":"hysteria","port":keep}]}).to_string(),
    )
    .unwrap();

    ConfigManager::delete_specific_configuration_at(cfg_path.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    // The unrelated range must still be locked: cleanup is content-driven, and
    // this config has no udpHop block.
    assert_eq!(xray_main_ports(&alloc), vec![keep]);
    assert!(!cfg_path.exists());
}

#[tokio::test]
async fn delete_of_a_non_hysteria_config_does_not_touch_xray_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    let (keep, _) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();

    // A Vision-style config must be inert to Hy2 hop cleanup. To pin the
    // `protocol == "hysteria"` guard INDEPENDENTLY of the `udpHop` guard, the
    // fixture carries a udpHop block (so `has_hop` is true) but a non-hysteria
    // protocol (so `is_hysteria` is false). Dropping only the protocol check
    // would release `keep` and fail this test.
    let cfg_path = dir.path().join("batch_reality_abcd1234_inbounds.json");
    std::fs::write(
        &cfg_path,
        json!({
            "inbounds":[{
                "protocol":"vless",
                "port":keep,
                "streamSettings":{
                    "finalmask":{"quicParams":{"udpHop":{"ports":format!("{}-{}", keep+1, keep+99)}}}
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    ConfigManager::delete_specific_configuration_at(cfg_path.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    assert_eq!(xray_main_ports(&alloc), vec![keep]);
}
