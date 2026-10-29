//! Contract: listing allowed ports must not silently drop range entries.
//!
//! Both backends write hop ranges (firewalld `"10001-10099/udp"`, ufw
//! `"20001:20100/udp"`), but the parsers only accepted a bare port number.
//! `parse::<u16>()` fails on `"10001-10099"`, so the entry vanished from the
//! returned set.
//!
//! Consequence: `sync_firewall_with_configs` computes `stale = current -
//! required`. With ranges missing from `current`, `stale` can never contain a
//! range, so `remove_port_range` is never called from the sync path and every
//! hop range ever created accumulates in the firewall forever. Measured on a
//! live node: 88 firewalld entries, only 43 survived parsing.

// The parsers are private; these tests exercise them through the same
// module-internal path the clients use.
use aegis::core::security::firewall::parse_port_entries;

#[test]
fn firewalld_range_entries_expand_to_every_port() {
    let ports = parse_port_entries("10300/tcp 10301-10399/udp");
    assert!(ports.contains(&10300), "plain port missing");
    for p in 10301..=10399u16 {
        assert!(
            ports.contains(&p),
            "range port {p} dropped: sync could never remove it"
        );
    }
    assert_eq!(ports.len(), 100);
}

#[test]
fn ufw_colon_ranges_are_parsed_too() {
    // ufw writes ranges with a colon, not a hyphen.
    let ports = parse_port_entries("20001:20100/udp");
    assert!(ports.contains(&20001), "range start missing");
    assert!(ports.contains(&20100), "range end missing");
    assert_eq!(ports.len(), 100);
}

#[test]
fn real_firewalld_output_shape() {
    // Verbatim shape from `firewall-cmd --list-ports` on a live node.
    let output = "10001-10099/udp 10101-10199/udp 10300/tcp 10300/udp 10301-10599/udp";
    let ports = parse_port_entries(output);
    assert!(ports.contains(&10001) && ports.contains(&10099));
    assert!(ports.contains(&10101) && ports.contains(&10199));
    assert!(ports.contains(&10300));
    assert!(ports.contains(&10301) && ports.contains(&10599));
    assert_eq!(ports.len(), 99 + 99 + 1 + 299);
}

#[test]
fn ranges_and_plain_ports_deduplicate() {
    let ports = parse_port_entries("10300/tcp 10300/udp 10299-10301/udp");
    assert_eq!(ports.len(), 3, "10300 appears three times, once is enough");
}

#[test]
fn malformed_entries_are_ignored_not_guessed() {
    for entry in [
        "abc/udp",
        "10399-10301/udp",
        "-/udp",
        "1-/udp",
        "-99/udp",
        "high/udp",
    ] {
        let ports = parse_port_entries(entry);
        assert!(
            ports.is_empty(),
            "entry {entry:?} must not invent ports, got {ports:?}"
        );
    }
}

#[test]
fn empty_input_yields_nothing() {
    assert!(parse_port_entries("").is_empty());
    assert!(parse_port_entries("   ").is_empty());
}
