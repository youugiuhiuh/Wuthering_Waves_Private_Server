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

#[tokio::test]
async fn hop_port_too_close_to_u16_max_is_ignored_not_panicked() {
    // The cleanup computes `main_port + 99` in u16 arithmetic. A config whose
    // port sits within 99 of u16::MAX would overflow: panic in debug, silent
    // wrap in release. Creation never writes such a port, but the value is read
    // from a file on disk, so the guard must bound it.
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    let cfg_path = dir
        .path()
        .join("batch_xray_hysteria2_overflow_inbounds.json");
    std::fs::write(
        &cfg_path,
        json!({
            "inbounds":[{
                "protocol":"hysteria",
                "port": 65535,
                "streamSettings":{
                    "finalmask":{"quicParams":{"udpHop":{"ports":"65536-65634"}}}
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    // Must not panic. The out-of-range port is skipped, so no cleanup occurs.
    ConfigManager::delete_specific_configuration_at(cfg_path.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    assert!(!cfg_path.exists(), "the file should still be deleted");
    assert_eq!(xray_main_ports(&alloc), Vec::<u16>::new());
}

#[tokio::test]
async fn hop_port_at_the_upper_bound_is_still_processed() {
    // Boundary counterpart: the highest port that CAN carry a 99-port hop range
    // must still be accepted, so the guard is not off by one.
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    let main: u16 = u16::MAX - 99; // 65436; +99 == 65535, the last valid hop end
    let (allocated, _) =
        aegis::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2_at(&alloc)
            .await
            .unwrap();
    // Seed the exact range we are about to delete.
    let data = serde_json::json!({
        "locked_ranges": [{
            "start": main, "end": u16::MAX, "protocol": "xray-hysteria2", "created_at": 0
        }],
        "initialized": true
    });
    std::fs::write(&alloc, data.to_string()).unwrap();
    assert_ne!(allocated, main, "sanity: allocator returned some port");

    let cfg_path = dir.path().join("batch_xray_hysteria2_upper_inbounds.json");
    std::fs::write(
        &cfg_path,
        json!({
            "inbounds":[{
                "protocol":"hysteria",
                "port": main,
                "streamSettings":{
                    "finalmask":{"quicParams":{"udpHop":{"ports":"65437-65535"}}}
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    ConfigManager::delete_specific_configuration_at(cfg_path.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    // The boundary port is in range, so its range must be released.
    assert_eq!(
        xray_main_ports(&alloc),
        Vec::<u16>::new(),
        "port {main} (u16::MAX - 99) is a valid hop main port and must be cleaned up"
    );
}

/// A bulk delete must release the range of a config it actually removed.
///
/// `delete_all_configurations` / `delete_configurations_by_count` read the
/// hardcoded `xray::CONF_DIR`, so their loops cannot be driven from a test.
/// What CAN be pinned is the shared decision function they now call, through
/// the observable per-config path: a config that is deleted has its range
/// released, and a config that is NOT deleted keeps it.
///
/// The ordering half of this contract — release only AFTER a successful
/// remove — is pinned by the companion test below.
#[tokio::test]
async fn deleting_a_config_releases_its_range_and_keeps_others() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");

    // Two hopping configs, each with its own locked range.
    let (first, _) =
        aegis::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2_at(&alloc)
            .await
            .unwrap();
    let (second, _) =
        aegis::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2_at(&alloc)
            .await
            .unwrap();

    let mk = |name: &str, port: u16| {
        let p = dir.path().join(name);
        std::fs::write(
            &p,
            serde_json::json!({
                "inbounds":[{
                    "protocol":"hysteria",
                    "port": port,
                    "streamSettings":{
                        "finalmask":{"quicParams":{"udpHop":{"ports":format!("{}-{}", port+1, port+99)}}}
                    }
                }]
            })
            .to_string(),
        )
        .unwrap();
        p
    };
    let first_cfg = mk("batch_xray_hysteria2_first_inbounds.json", first);

    ConfigManager::delete_specific_configuration_at(first_cfg.to_str().unwrap(), Some(&alloc))
        .await
        .unwrap();

    // The deleted config's range is gone; the surviving config keeps its own.
    assert_eq!(
        xray_main_ports(&alloc),
        vec![second],
        "deleting one config must release exactly its own range"
    );
    assert!(!first_cfg.exists());
}

/// A delete whose `remove_file` FAILS must not release the range.
///
/// This is the ordering half of the delete contract, and the bug the final
/// review caught: `delete_specific_configuration_at` must extract → remove →
/// cleanup, and bail before cleanup when the removal fails. Releasing first
/// would free the ports of a config that is still live, so the next allocation
/// could overlap a running listener.
///
/// The failure is forced by making the containing directory read-only, so the
/// config file itself stays READABLE (its ports extract normally) while the
/// unlink fails — a directory-as-file mock would make extraction return an
/// empty list and the test would pass under either ordering.
#[tokio::test]
async fn failed_delete_must_not_release_the_range() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    let (main, _) =
        aegis::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2_at(&alloc)
            .await
            .unwrap();

    let sub = dir.path().join("conf");
    std::fs::create_dir(&sub).unwrap();
    let cfg = sub.join("batch_xray_hysteria2_ro_inbounds.json");
    std::fs::write(
        &cfg,
        serde_json::json!({
            "inbounds":[{
                "protocol":"hysteria",
                "port": main,
                "streamSettings":{
                    "finalmask":{"quicParams":{"udpHop":{"ports":format!("{}-{}", main+1, main+99)}}}
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    // Make the directory read-only so unlink fails but the file stays readable.
    let mut perms = std::fs::metadata(&sub).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(&sub, perms).unwrap();

    let result =
        ConfigManager::delete_specific_configuration_at(cfg.to_str().unwrap(), Some(&alloc)).await;

    // Restore permissions so the tempdir can be cleaned up.
    let mut perms = std::fs::metadata(&sub).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&sub, perms).unwrap();

    // Skip rather than fail when running as root: root ignores the mode bits
    // and the unlink would succeed, so the scenario cannot be constructed.
    if result.is_ok() {
        assert!(
            !cfg.exists(),
            "if the delete succeeded the file must be gone"
        );
        eprintln!("skipped ordering assertion: running as root, unlink not blocked");
        return;
    }

    assert!(cfg.exists(), "the config file must still be present");
    assert_eq!(
        xray_main_ports(&alloc),
        vec![main],
        "a FAILED delete must NOT release range {main}; releasing first would \
         free the ports of a live config"
    );
}
