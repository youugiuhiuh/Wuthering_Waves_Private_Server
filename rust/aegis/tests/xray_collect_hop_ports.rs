//! Contract: `collect_all_ports` must report every port a config needs open.
//!
//! The regression: it read only the inbound's `port`, so a hopping hysteria2
//! config reported `10300` but not `10301-10399`. `sync_firewall_with_configs`
//! then treats anything absent from the collected set as stale and removes it,
//! so every sync deleted the allowances of a live hopping config.
//!
//! sing-box's `extract_ports_recursive` already includes its hop range; the
//! Xray path must match or the two cores disagree inside the same sync.

use aegis::core::xray::ConfigManager;
use serde_json::json;
use std::collections::HashSet;

fn collect(cfg: serde_json::Value) -> HashSet<u16> {
    let mut ports = HashSet::new();
    ConfigManager::collect_ports_from_config(&cfg, &mut ports);
    ports
}

fn hopping_config(main: u16, start: u16, end: u16) -> serde_json::Value {
    json!({
        "inbounds": [{
            "protocol": "hysteria",
            "port": main,
            "streamSettings": {
                "finalmask": { "quicParams": { "udpHop": { "ports": format!("{start}-{end}") } } }
            }
        }]
    })
}

#[test]
fn hopping_config_contributes_its_whole_range() {
    let ports = collect(hopping_config(10300, 10301, 10399));

    assert!(ports.contains(&10300), "main port missing");
    for p in 10301..=10399u16 {
        assert!(
            ports.contains(&p),
            "hop port {p} missing: sync would treat it as stale and remove it"
        );
    }
    assert_eq!(ports.len(), 100, "expected main + 99 hop ports");
}

#[test]
fn plain_config_contributes_only_its_port() {
    let ports = collect(json!({"inbounds": [{"protocol": "vless", "port": 443}]}));
    assert_eq!(ports, HashSet::from([443]));
}

#[test]
fn hop_range_uses_the_declared_bounds_not_main_plus_99() {
    // A narrower range must not be widened: whitelisting ports nothing listens
    // on would open holes for no reason.
    let ports = collect(hopping_config(20000, 20005, 20010));
    assert_eq!(ports.len(), 7, "main + 6 hop ports");
    assert!(ports.contains(&20005) && ports.contains(&20010));
    assert!(
        !ports.contains(&20011),
        "range must stop at the declared end"
    );
    assert!(
        !ports.contains(&20004),
        "range must start at the declared start"
    );
}

#[test]
fn malformed_or_reversed_hop_specs_fall_back_to_the_main_port() {
    for spec in ["", "abc", "100", "10399-10301", "-", "1-", "-99"] {
        let cfg = json!({
            "inbounds": [{
                "protocol": "hysteria",
                "port": 10300,
                "streamSettings": { "finalmask": { "quicParams": { "udpHop": { "ports": spec } } } }
            }]
        });
        let ports = collect(cfg);
        assert_eq!(
            ports,
            HashSet::from([10300]),
            "spec {spec:?} must not invent a range"
        );
    }
}

#[test]
fn multiple_inbounds_accumulate() {
    let cfg = json!({"inbounds": [
        {"protocol": "hysteria", "port": 10300,
         "streamSettings": {"finalmask": {"quicParams": {"udpHop": {"ports": "10301-10302"}}}}},
        {"protocol": "hysteria", "port": 10400,
         "streamSettings": {"finalmask": {"quicParams": {"udpHop": {"ports": "10401-10402"}}}}},
        {"protocol": "vless", "port": 443}
    ]});
    let ports = collect(cfg);
    assert_eq!(
        ports,
        HashSet::from([443, 10300, 10301, 10302, 10400, 10401, 10402])
    );
}
