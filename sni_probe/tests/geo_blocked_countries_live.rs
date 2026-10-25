//! 用真实 ipinfo_lite.mmdb 验证 CN/RU/IR 等管辖国确实被丢弃。
//! 需要 MMDB 文件存在，否则跳过（CI 无该文件时不失败）。

use std::net::IpAddr;

#[path = "../src/geo.rs"]
#[allow(dead_code)]
mod geo;

fn mmdb_path() -> Option<String> {
    for p in [
        "../sni_tester/ipinfo_lite.mmdb",
        "ipinfo_lite.mmdb",
        "/tmp/ipinfo_lite.mmdb",
    ] {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

#[test]
fn real_db_classifies_censoring_countries_as_blocked() {
    let Some(path) = mmdb_path() else {
        eprintln!("跳过：未找到 ipinfo_lite.mmdb");
        return;
    };
    let db = geo::GeoDb::open(&path).expect("应能打开 MMDB");

    // 各国公开 DNS / 知名主机，覆盖 CN / IR / RU / US / DE
    let cases: &[(&str, bool)] = &[
        ("114.114.114.114", true), // CN 南京信风
        ("223.5.5.5", true),       // CN 阿里
        ("217.218.155.155", true), // IR
        ("77.88.8.8", true),       // RU Yandex
        ("8.8.8.8", false),        // US Google
        ("144.76.0.1", false),     // DE Hetzner
    ];

    let mut checked = 0;
    for (ip, want_blocked) in cases {
        let addr: IpAddr = ip.parse().unwrap();
        let country = db.country(addr);
        if country == "UNKNOWN" {
            continue; // Lite 库对部分地址无记录，不算失败
        }
        eprintln!(
            "{ip:20} -> {country:8} blocked={}",
            geo::is_blocked_country(&country)
        );
        assert_eq!(
            geo::is_blocked_country(&country),
            *want_blocked,
            "{ip} 解析为 {country}，期望 blocked={want_blocked}"
        );
        checked += 1;
    }
    assert!(checked >= 4, "有效样本太少（{checked}），MMDB 可能不完整");
}
