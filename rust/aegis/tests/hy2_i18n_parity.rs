//! Every hysteria2 i18n key must be present and translated in all supported
//! locales.
//!
//! Asserting on the raw YAML (not `t!()`) is deliberate: `t!()` falls back to
//! the key name and then to English, so a missing Japanese key or an
//! untranslated English string in `zh.yml` renders "fine" and cannot be caught
//! through the lookup API. The files are the contract.

use std::collections::HashMap;
use std::fs;

/// Parse the flat `key: "value"` subset of the locale files into dotted keys.
///
/// Two levels only (`xray:` / `  hy2_obfs_title:`), which is all these files
/// use for the hy2 blocks.
fn parse_locale(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if indent == 0 {
            section = key.to_string();
        } else if !section.is_empty() && !value.is_empty() {
            out.insert(format!("{section}.{key}"), value.to_string());
        }
    }
    out
}

fn locale(name: &str) -> HashMap<String, String> {
    let path = format!(
        "{}/src/resources/i18n/{name}.yml",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    parse_locale(&text)
}

/// Xray hy2 keys reachable from the batch flow, including the two step titles
/// that were previously hardcoded in `xray.rs`.
const XRAY_HY2_KEYS: &[&str] = &[
    "xray.hy2_obfs_title",
    "xray.hy2_obfs_none",
    "xray.hy2_obfs_salamander",
    "xray.hy2_obfs_gecko",
    "xray.hy2_hop_title",
    "xray.hy2_hop_off",
    "xray.hy2_hop_on_official",
    "xray.hy2_hop_on_v2rayn",
    "xray.hy2_obfs_step_title",
    "xray.hy2_hop_step_title",
];

/// sing-box hy2 keys; the hop step offers two link styles and the labels are
/// looked up by name.
const SINGBOX_HY2_KEYS: &[&str] = &[
    "menu.singbox_h2_hop_enable_singbox",
    "menu.singbox_h2_hop_enable_v2rayn",
];

#[test]
fn every_hy2_key_exists_in_all_locales() {
    let en = locale("en");
    let zh = locale("zh");
    let ja = locale("ja");

    let mut missing = Vec::new();
    for key in XRAY_HY2_KEYS.iter().chain(SINGBOX_HY2_KEYS) {
        for (name, map) in [("en", &en), ("zh", &zh), ("ja", &ja)] {
            if !map.contains_key(*key) {
                missing.push(format!("{key} missing in {name}.yml"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "untranslated/missing hy2 keys:\n{}",
        missing.join("\n")
    );
}

/// Values allowed to stay identical to English in CJK locales: brand names
/// with no translatable words, and step-title shells that are mostly the
/// product name plus positional placeholders (`Hysteria2 (Xray) | %{0}`).
const BRAND_ONLY_OR_TEMPLATE: &[&str] = &[
    "xray.hy2_obfs_salamander",
    "xray.hy2_obfs_gecko",
    "xray.hy2_obfs_step_title",
    "xray.hy2_hop_step_title",
];

#[test]
fn cjk_locales_are_not_left_in_english() {
    let en = locale("en");
    let zh = locale("zh");
    let ja = locale("ja");

    // A CJK locale whose value is byte-identical to English is an untranslated
    // string, unless the value is a brand name or a placeholder template.
    let all_keys: Vec<&str> = XRAY_HY2_KEYS
        .iter()
        .chain(SINGBOX_HY2_KEYS)
        .copied()
        .collect();
    let mut untranslated = Vec::new();

    for key in all_keys {
        if BRAND_ONLY_OR_TEMPLATE.contains(&key) {
            continue;
        }
        let Some(en_val) = en.get(key) else { continue };
        // Strip the leading emoji(s) so "🚫 None" still counts as text.
        let en_words: String = en_val
            .chars()
            .filter(|c| c.is_ascii_alphabetic() || c.is_whitespace())
            .collect();
        if en_words.trim().is_empty() {
            continue; // emoji-only label, nothing to translate
        }
        for (name, map) in [("zh", &zh), ("ja", &ja)] {
            if map.get(key) == Some(en_val) {
                untranslated.push(format!("{key} is still English in {name}.yml: {en_val}"));
            }
        }
    }

    assert!(
        untranslated.is_empty(),
        "untranslated hy2 strings:\n{}",
        untranslated.join("\n")
    );
}
