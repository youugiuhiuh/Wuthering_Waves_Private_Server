use std::collections::HashMap;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn yaml_files() -> Vec<(String, PathBuf)> {
    let i18n = manifest_dir().join("src").join("resources").join("i18n");
    vec![
        ("en".to_string(), i18n.join("en.yml")),
        ("zh".to_string(), i18n.join("zh.yml")),
        ("ja".to_string(), i18n.join("ja.yml")),
    ]
}

fn flatten_keys(prefix: &str, value: &serde_yaml::Value, out: &mut HashMap<String, String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                let key = k.as_str().map(String::from).unwrap_or_default();
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match v {
                    serde_yaml::Value::Mapping(_) => flatten_keys(&full_key, v, out),
                    _ => {
                        out.insert(full_key, v.as_str().unwrap_or("").to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

fn load_keys(path: &PathBuf) -> HashMap<String, String> {
    let content = std::fs::read_to_string(path).expect("yaml file must be readable");
    let root: serde_yaml::Value = serde_yaml::from_str(&content).expect("valid yaml");
    let mut keys = HashMap::new();
    flatten_keys("", &root, &mut keys);
    keys
}

#[test]
fn all_en_subscription_keys_exist_in_zh_and_ja() {
    let files = yaml_files();
    let en_path = files
        .iter()
        .find(|(lang, _)| lang == "en")
        .map(|(_, path)| path)
        .expect("en.yml must exist");
    let zh_path = files
        .iter()
        .find(|(lang, _)| lang == "zh")
        .map(|(_, path)| path)
        .expect("zh.yml must exist");
    let ja_path = files
        .iter()
        .find(|(lang, _)| lang == "ja")
        .map(|(_, path)| path)
        .expect("ja.yml must exist");

    let en_keys = load_keys(en_path);
    let zh_keys = load_keys(zh_path);
    let ja_keys = load_keys(ja_path);

    let mut missing = Vec::new();

    let subscription_key_prefixes = &["subscription.", "menu.subscription"];

    for (key, _) in &en_keys {
        let is_subscription_key = subscription_key_prefixes
            .iter()
            .any(|prefix| key.starts_with(prefix));
        if !is_subscription_key {
            continue;
        }
        if !zh_keys.contains_key(key) {
            missing.push(format!("  {key} — missing in zh.yml"));
        }
        if !ja_keys.contains_key(key) {
            missing.push(format!("  {key} — missing in ja.yml"));
        }
    }

    if !missing.is_empty() {
        panic!(
            "{} subscription English keys missing in other locales:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }
}

#[test]
fn subscription_keys_present_in_all_locales() {
    let files = yaml_files();
    let required_subscription_keys = &[
        "subscription.title",
        "subscription.status",
        "subscription.enabled_status",
        "subscription.disabled_status",
        "subscription.enable",
        "subscription.disable",
        "subscription.not_init",
        "subscription.not_set",
        "subscription.mode_label",
        "subscription.mode_domain",
        "subscription.mode_ip",
        "subscription.host_label",
        "subscription.port_label",
        "subscription.masked_token_label",
        "subscription.cert_label",
        "subscription.cert_valid",
        "subscription.cert_none",
        "subscription.cert_reissuing",
        "subscription.cert_fail",
        "subscription.last_error",
        "subscription.refresh",
        "subscription.set_domain",
        "subscription.set_ip",
        "subscription.set_ipv6_san",
        "subscription.set_port",
        "subscription.regenerate_token_btn",
        "subscription.reissue_cert_btn",
        "subscription.toggle_success",
        "subscription.toggle_fail",
        "subscription.input_domain_prompt",
        "subscription.input_ip_prompt",
        "subscription.input_ipv6_san_prompt",
        "subscription.input_port_prompt",
        "subscription.input_timeout_hint",
        "subscription.input_timeout",
        "subscription.invalid_input",
        "subscription.invalid_ip",
        "subscription.invalid_ipv6",
        "subscription.invalid_port",
        "subscription.invalid_port_80",
        "subscription.config_updated",
        "subscription.config_fail",
        "subscription.token_regenerated_msg",
        "subscription.token_regenerated_cb",
        "subscription.token_fail",
        "menu.subscription",
    ];

    for (lang, path) in &files {
        let keys = load_keys(path);
        for required in required_subscription_keys {
            assert!(
                keys.contains_key(*required),
                "{lang}: missing key {required}",
            );
        }
    }

    // Verify subscription.input_empty was removed (dead key)
    for (lang, path) in &files {
        let keys = load_keys(path);
        assert!(
            !keys.contains_key("subscription.input_empty"),
            "{lang}: subscription.input_empty should have been removed",
        );
    }
}
