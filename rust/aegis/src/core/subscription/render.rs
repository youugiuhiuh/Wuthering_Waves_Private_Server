use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use super::node::{Hysteria2Node, SubscriptionNode, TuicNode, VlessNetwork, VlessRealityNode};

pub fn render_base64(nodes: &[SubscriptionNode], host: &str) -> Result<String> {
    let links = nodes
        .iter()
        .map(|node| render_link(node, host))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(STANDARD.encode(links))
}

pub fn render_clash(nodes: &[SubscriptionNode], host: &str) -> Result<String> {
    let mut yaml = String::from("proxies:\n");
    for node in nodes {
        yaml.push_str(&match node {
            SubscriptionNode::VlessReality(node) => render_vless_yaml(node, host),
            SubscriptionNode::Hysteria2(node) => render_hysteria2_yaml(node, host),
            SubscriptionNode::Tuic(node) => render_tuic_yaml(node, host),
        });
    }
    Ok(yaml)
}

fn render_link(node: &SubscriptionNode, host: &str) -> String {
    match node {
        SubscriptionNode::VlessReality(node) => render_vless_link(node, host),
        SubscriptionNode::Hysteria2(node) => render_hysteria2_link(node, host),
        SubscriptionNode::Tuic(node) => render_tuic_link(node, host),
    }
}

fn render_vless_link(node: &VlessRealityNode, host: &str) -> String {
    let sni = percent_encode(&node.server_name);
    let public_key = percent_encode(&node.public_key);
    let name = percent_encode(&node.name);
    match &node.network {
        VlessNetwork::Tcp => format!(
            "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={sni}&fp=chrome&pbk={public_key}&sid={}&type=tcp&headerType=none#{name}",
            node.uuid, host, node.port, node.short_id
        ),
        VlessNetwork::Xhttp { path } => format!(
            "vless://{}@{}:{}?encryption=none&security=reality&sni={sni}&fp=chrome&pbk={public_key}&sid={}&type=xhttp&path={}&mode=auto#{name}",
            node.uuid,
            host,
            node.port,
            node.short_id,
            percent_encode(path)
        ),
    }
}

fn render_hysteria2_link(node: &Hysteria2Node, host: &str) -> String {
    let mut link = format!(
        "hysteria2://{}@{}:{}?sni={}&alpn={}&pinSHA256={}",
        percent_encode(&node.password),
        host,
        node.port,
        percent_encode(&node.server_name),
        node.alpn.join(","),
        node.cert_fingerprint
    );
    if let (Some(obfs), Some(password)) = (&node.obfs, &node.obfs_password) {
        link.push_str(&format!(
            "&obfs={}&obfs-password={}",
            percent_encode(obfs),
            percent_encode(password)
        ));
    }
    link.push('#');
    link.push_str(&percent_encode(&node.name));
    link
}

fn render_tuic_link(node: &TuicNode, host: &str) -> String {
    format!(
        "tuic://{}:{}@{}:{}?sni={}&alpn={}&congestion_control={}&pcs={}#{}",
        node.uuid,
        percent_encode(&node.password),
        host,
        node.port,
        percent_encode(&node.server_name),
        node.alpn.join(","),
        node.congestion_control,
        node.cert_fingerprint,
        percent_encode(&node.name)
    )
}

fn render_vless_yaml(node: &VlessRealityNode, host: &str) -> String {
    let mut yaml = format!(
        concat!(
            "  - name: {}\n",
            "    type: vless\n",
            "    server: {}\n",
            "    port: {}\n",
            "    uuid: {}\n",
            "    tls: true\n",
            "    servername: {}\n",
            "    client-fingerprint: chrome\n",
            "    reality-opts:\n",
            "      public-key: {}\n",
            "      short-id: {}\n"
        ),
        yaml_quote(&node.name),
        yaml_quote(host),
        node.port,
        yaml_quote(&node.uuid),
        yaml_quote(&node.server_name),
        yaml_quote(&node.public_key),
        yaml_quote(&node.short_id)
    );
    match &node.network {
        VlessNetwork::Tcp => yaml.push_str("    flow: xtls-rprx-vision\n"),
        VlessNetwork::Xhttp { path } => yaml.push_str(&format!(
            "    network: xhttp\n    xhttp-opts:\n      path: {}\n",
            yaml_quote(path)
        )),
    }
    yaml
}

fn render_hysteria2_yaml(node: &Hysteria2Node, host: &str) -> String {
    let mut yaml = format!(
        concat!(
            "  - name: {}\n",
            "    type: hysteria2\n",
            "    server: {}\n",
            "    port: {}\n",
            "    password: {}\n",
            "    sni: {}\n",
            "    alpn:\n{}"
        ),
        yaml_quote(&node.name),
        yaml_quote(host),
        node.port,
        yaml_quote(&node.password),
        yaml_quote(&node.server_name),
        render_alpn(&node.alpn)
    );
    if let (Some(obfs), Some(password)) = (&node.obfs, &node.obfs_password) {
        yaml.push_str(&format!(
            "    obfs: {}\n    obfs-password: {}\n",
            yaml_plain(obfs),
            yaml_quote(password)
        ));
    }
    yaml.push_str(&format!(
        "    fingerprint: {}\n",
        yaml_quote(&mihomo_fingerprint(&node.cert_fingerprint))
    ));
    yaml
}

fn render_tuic_yaml(node: &TuicNode, host: &str) -> String {
    format!(
        concat!(
            "  - name: {}\n",
            "    type: tuic\n",
            "    server: {}\n",
            "    port: {}\n",
            "    uuid: {}\n",
            "    password: {}\n",
            "    sni: {}\n",
            "    alpn:\n{}",
            "    congestion-controller: {}\n",
            "    udp-relay-mode: native\n",
            "    fingerprint: {}\n"
        ),
        yaml_quote(&node.name),
        yaml_quote(host),
        node.port,
        yaml_quote(&node.uuid),
        yaml_quote(&node.password),
        yaml_quote(&node.server_name),
        render_alpn(&node.alpn),
        yaml_plain(&node.congestion_control),
        yaml_quote(&mihomo_fingerprint(&node.cert_fingerprint))
    )
}

fn render_alpn(alpn: &[String]) -> String {
    alpn.iter()
        .map(|value| format!("      - {}\n", yaml_plain(value)))
        .collect()
}

fn percent_encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn mihomo_fingerprint(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            let separator = (index > 0 && index.is_multiple_of(2)).then_some(':');
            separator.into_iter().chain(character.to_uppercase())
        })
        .collect()
}

fn yaml_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\r' => quoted.push_str("\\r"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn yaml_plain(value: &str) -> String {
    let safe = value.starts_with(|character: char| character.is_ascii_alphabetic())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "yes" | "no" | "on" | "off"
        );
    if safe {
        value.to_owned()
    } else {
        yaml_quote(value)
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{render_base64, render_clash};
    use crate::core::subscription::node::{
        Hysteria2Node, SubscriptionNode, TuicNode, VlessNetwork, VlessRealityNode,
    };

    const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const CERT_FINGERPRINT: &str =
        "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const MIHOMO_FINGERPRINT: &str = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";

    #[test]
    fn base64_decodes_to_exact_existing_link_formats() {
        let encoded = render_base64(&all_node_variants(), "203.0.113.10").unwrap();
        let decoded = STANDARD.decode(encoded).unwrap();
        let text = String::from_utf8(decoded).unwrap();

        assert_eq!(
            text,
            concat!(
                "vless://123e4567-e89b-12d3-a456-426614174000@203.0.113.10:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=cdn%2Eexample%2Ecom&fp=chrome&pbk=public%2Dkey%5F123&sid=0123456789abcdef&type=tcp&headerType=none#Reality%20Vision\n",
                "vless://123e4567-e89b-12d3-a456-426614174000@203.0.113.10:8443?encryption=none&security=reality&sni=cdn%2Eexample%2Ecom&fp=chrome&pbk=public%2Dkey%5F123&sid=fedcba9876543210&type=xhttp&path=%2Fassets%2Fupload&mode=auto#Reality%20XHTTP\n",
                "hysteria2://hy%2Dsecret@203.0.113.10:9443?sni=hy%2Eexample%2Ecom&alpn=h3&pinSHA256=00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff&obfs=salamander&obfs-password=obfs%2Dsecret#Hysteria%202\n",
                "tuic://123e4567-e89b-12d3-a456-426614174000:tuic%2Dsecret@203.0.113.10:10443?sni=tuic%2Eexample%2Ecom&alpn=h3&congestion_control=bbr&pcs=00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff#TUIC"
            )
        );
    }

    #[test]
    fn clash_yaml_exactly_matches_current_mihomo_fields() {
        let yaml = render_clash(&all_node_variants(), "sub.example.com").unwrap();

        assert_eq!(
            yaml,
            format!(
                concat!(
                    "proxies:\n",
                    "  - name: \"Reality Vision\"\n",
                    "    type: vless\n",
                    "    server: \"sub.example.com\"\n",
                    "    port: 443\n",
                    "    uuid: \"123e4567-e89b-12d3-a456-426614174000\"\n",
                    "    tls: true\n",
                    "    servername: \"cdn.example.com\"\n",
                    "    client-fingerprint: chrome\n",
                    "    reality-opts:\n",
                    "      public-key: \"public-key_123\"\n",
                    "      short-id: \"0123456789abcdef\"\n",
                    "    flow: xtls-rprx-vision\n",
                    "  - name: \"Reality XHTTP\"\n",
                    "    type: vless\n",
                    "    server: \"sub.example.com\"\n",
                    "    port: 8443\n",
                    "    uuid: \"123e4567-e89b-12d3-a456-426614174000\"\n",
                    "    tls: true\n",
                    "    servername: \"cdn.example.com\"\n",
                    "    client-fingerprint: chrome\n",
                    "    reality-opts:\n",
                    "      public-key: \"public-key_123\"\n",
                    "      short-id: \"fedcba9876543210\"\n",
                    "    network: xhttp\n",
                    "    xhttp-opts:\n",
                    "      path: \"/assets/upload\"\n",
                    "  - name: \"Hysteria 2\"\n",
                    "    type: hysteria2\n",
                    "    server: \"sub.example.com\"\n",
                    "    port: 9443\n",
                    "    password: \"hy-secret\"\n",
                    "    sni: \"hy.example.com\"\n",
                    "    alpn:\n",
                    "      - h3\n",
                    "    obfs: salamander\n",
                    "    obfs-password: \"obfs-secret\"\n",
                    "    fingerprint: \"{}\"\n",
                    "  - name: \"TUIC\"\n",
                    "    type: tuic\n",
                    "    server: \"sub.example.com\"\n",
                    "    port: 10443\n",
                    "    uuid: \"123e4567-e89b-12d3-a456-426614174000\"\n",
                    "    password: \"tuic-secret\"\n",
                    "    sni: \"tuic.example.com\"\n",
                    "    alpn:\n",
                    "      - h3\n",
                    "    congestion-controller: bbr\n",
                    "    udp-relay-mode: native\n",
                    "    fingerprint: \"{}\"\n"
                ),
                MIHOMO_FINGERPRINT, MIHOMO_FINGERPRINT
            )
        );
        assert!(!yaml.contains("skip-cert-verify"));
    }

    #[test]
    fn clash_yaml_quotes_and_escapes_untrusted_scalars() {
        let node = SubscriptionNode::VlessReality(VlessRealityNode {
            name: "node: \\\"quoted\\\"\r\n\t\\".to_owned(),
            port: 443,
            uuid: "uuid:\n".to_owned(),
            server_name: "sni:\r\n".to_owned(),
            public_key: "key:\"\\".to_owned(),
            short_id: "id:\t".to_owned(),
            flow: None,
            network: VlessNetwork::Xhttp {
                path: "/path:\r\n\t\\\"".to_owned(),
            },
        });

        let yaml = render_clash(&[node], "host:\r\n\t\\\"").unwrap();

        assert!(yaml.contains("name: \"node: \\\\\\\"quoted\\\\\\\"\\r\\n\\t\\\\\"\n"));
        assert!(yaml.contains("server: \"host:\\r\\n\\t\\\\\\\"\"\n"));
        assert!(yaml.contains("uuid: \"uuid:\\n\"\n"));
        assert!(yaml.contains("servername: \"sni:\\r\\n\"\n"));
        assert!(yaml.contains("public-key: \"key:\\\"\\\\\"\n"));
        assert!(yaml.contains("short-id: \"id:\\t\"\n"));
        assert!(yaml.contains("path: \"/path:\\r\\n\\t\\\\\\\"\"\n"));
    }

    #[test]
    fn clash_yaml_quotes_dynamic_fields_when_plain_yaml_would_be_unsafe() {
        let nodes = vec![
            SubscriptionNode::Hysteria2(Hysteria2Node {
                name: "Hysteria 2".to_owned(),
                port: 443,
                password: "password".to_owned(),
                server_name: "example.com".to_owned(),
                alpn: vec!["h3\nmalicious: true".to_owned()],
                obfs: Some("salamander\nmalicious: true".to_owned()),
                obfs_password: Some("password".to_owned()),
                cert_fingerprint: CERT_FINGERPRINT.to_owned(),
            }),
            SubscriptionNode::Tuic(TuicNode {
                name: "TUIC".to_owned(),
                port: 8443,
                uuid: UUID.to_owned(),
                password: "password".to_owned(),
                server_name: "example.com".to_owned(),
                alpn: vec!["h3".to_owned()],
                congestion_control: "bbr\nmalicious: true".to_owned(),
                cert_fingerprint: CERT_FINGERPRINT.to_owned(),
            }),
        ];

        let yaml = render_clash(&nodes, "example.com").unwrap();

        assert!(yaml.contains("- \"h3\\nmalicious: true\"\n"));
        assert!(yaml.contains("obfs: \"salamander\\nmalicious: true\"\n"));
        assert!(yaml.contains("congestion-controller: \"bbr\\nmalicious: true\"\n"));
        assert!(!yaml.contains("\nmalicious: true\n"));
    }

    fn all_node_variants() -> Vec<SubscriptionNode> {
        vec![
            SubscriptionNode::VlessReality(VlessRealityNode {
                name: "Reality Vision".to_owned(),
                port: 443,
                uuid: UUID.to_owned(),
                server_name: "cdn.example.com".to_owned(),
                public_key: "public-key_123".to_owned(),
                short_id: "0123456789abcdef".to_owned(),
                flow: Some("xtls-rprx-vision".to_owned()),
                network: VlessNetwork::Tcp,
            }),
            SubscriptionNode::VlessReality(VlessRealityNode {
                name: "Reality XHTTP".to_owned(),
                port: 8443,
                uuid: UUID.to_owned(),
                server_name: "cdn.example.com".to_owned(),
                public_key: "public-key_123".to_owned(),
                short_id: "fedcba9876543210".to_owned(),
                flow: None,
                network: VlessNetwork::Xhttp {
                    path: "/assets/upload".to_owned(),
                },
            }),
            SubscriptionNode::Hysteria2(Hysteria2Node {
                name: "Hysteria 2".to_owned(),
                port: 9443,
                password: "hy-secret".to_owned(),
                server_name: "hy.example.com".to_owned(),
                alpn: vec!["h3".to_owned()],
                obfs: Some("salamander".to_owned()),
                obfs_password: Some("obfs-secret".to_owned()),
                cert_fingerprint: CERT_FINGERPRINT.to_owned(),
            }),
            SubscriptionNode::Tuic(TuicNode {
                name: "TUIC".to_owned(),
                port: 10443,
                uuid: UUID.to_owned(),
                password: "tuic-secret".to_owned(),
                server_name: "tuic.example.com".to_owned(),
                alpn: vec!["h3".to_owned()],
                congestion_control: "bbr".to_owned(),
                cert_fingerprint: CERT_FINGERPRINT.to_owned(),
            }),
        ]
    }
}
