use std::{fs, path::Path};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionNode {
    VlessReality(VlessRealityNode),
    Hysteria2(Hysteria2Node),
    Tuic(TuicNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VlessRealityNode {
    pub name: String,
    pub port: u16,
    pub uuid: String,
    pub server_name: String,
    pub public_key: String,
    pub short_id: String,
    pub flow: Option<String>,
    pub network: VlessNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VlessNetwork {
    Tcp,
    Xhttp { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hysteria2Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuicNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeLoad {
    pub nodes: Vec<SubscriptionNode>,
    pub skipped: usize,
    diagnostics: Vec<String>,
}

impl NodeLoad {
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn require_nodes(self) -> Result<Self> {
        if self.nodes.is_empty() {
            return Err(anyhow!("no supported subscription nodes found"));
        }
        Ok(self)
    }
}

pub fn load_xray_nodes(config_dir: &Path) -> Result<NodeLoad> {
    let mut paths = fs::read_dir(config_dir)
        .context("failed to read Xray config directory")?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    paths.sort();

    let mut load = NodeLoad {
        nodes: Vec::new(),
        skipped: 0,
        diagnostics: Vec::new(),
    };
    for path in paths {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<non-utf8>.json");
        let Ok(raw) = fs::read(&path) else {
            load.skipped += 1;
            load.diagnostics.push(format!("{filename}: read-error"));
            continue;
        };
        let Ok(config) = serde_json::from_slice::<Value>(&raw) else {
            load.skipped += 1;
            load.diagnostics.push(format!("{filename}: malformed-json"));
            continue;
        };
        let Some(inbounds) = config.get("inbounds").and_then(Value::as_array) else {
            load.skipped += 1;
            load.diagnostics.push(format!("{filename}: malformed-json"));
            continue;
        };
        for (index, inbound) in inbounds.iter().enumerate() {
            match parse_reality_inbound(inbound) {
                Ok(node) => load.nodes.push(SubscriptionNode::VlessReality(node)),
                Err(reason) => {
                    load.skipped += 1;
                    load.diagnostics
                        .push(format!("{filename} inbound[{index}]: {}", reason.as_str()));
                }
            }
        }
    }
    Ok(load)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundFailure {
    Unsupported,
    Malformed,
}

impl InboundFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Malformed => "malformed",
        }
    }
}

fn parse_reality_inbound(inbound: &Value) -> std::result::Result<VlessRealityNode, InboundFailure> {
    use InboundFailure::Malformed;

    match inbound.get("protocol").and_then(Value::as_str) {
        Some("vless") => {}
        Some(_) => return Err(InboundFailure::Unsupported),
        None => return Err(InboundFailure::Malformed),
    }
    let stream = inbound
        .get("streamSettings")
        .ok_or(InboundFailure::Malformed)?;
    match stream.get("security").and_then(Value::as_str) {
        Some("reality") => {}
        Some(_) => return Err(InboundFailure::Unsupported),
        None => return Err(InboundFailure::Malformed),
    }
    let client = inbound
        .get("settings")
        .and_then(|settings| settings.get("clients"))
        .and_then(Value::as_array)
        .and_then(|clients| clients.first())
        .ok_or(Malformed)?;
    let flow = client
        .get("flow")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let network = match stream.get("network").and_then(Value::as_str) {
        Some("tcp") if flow.as_deref() == Some("xtls-rprx-vision") => VlessNetwork::Tcp,
        Some("xhttp") if flow.is_none() => VlessNetwork::Xhttp {
            path: stream
                .get("xhttpSettings")
                .and_then(|settings| settings.get("path"))
                .and_then(Value::as_str)
                .ok_or(Malformed)?
                .to_owned(),
        },
        Some(_) => return Err(InboundFailure::Unsupported),
        None => return Err(InboundFailure::Malformed),
    };
    let reality = stream.get("realitySettings").ok_or(Malformed)?;
    let private_key = URL_SAFE_NO_PAD
        .decode(
            reality
                .get("privateKey")
                .and_then(Value::as_str)
                .ok_or(Malformed)?,
        )
        .map_err(|_| Malformed)?;
    let private_key: [u8; 32] = private_key.try_into().map_err(|_| Malformed)?;
    let public_key = PublicKey::from(&StaticSecret::from(private_key));
    let server_name = reality
        .get("serverNames")
        .and_then(Value::as_array)
        .and_then(|names| names.first())
        .and_then(Value::as_str)
        .ok_or(Malformed)?;
    let short_id = reality
        .get("shortIds")
        .and_then(Value::as_array)
        .and_then(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .find(|id| !id.is_empty())
        })
        .ok_or(Malformed)?;

    Ok(VlessRealityNode {
        name: client
            .get("email")
            .and_then(Value::as_str)
            .ok_or(Malformed)?
            .to_owned(),
        port: inbound
            .get("port")
            .and_then(Value::as_u64)
            .ok_or(Malformed)?
            .try_into()
            .map_err(|_| Malformed)?,
        uuid: client
            .get("id")
            .and_then(Value::as_str)
            .ok_or(Malformed)?
            .to_owned(),
        server_name: server_name.to_owned(),
        public_key: URL_SAFE_NO_PAD.encode(public_key.as_bytes()),
        short_id: short_id.to_owned(),
        flow,
        network,
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::{Value, json};

    use super::{SubscriptionNode, VlessNetwork, load_xray_nodes};

    const FIXTURE_UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const FIXTURE_PRIVATE_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const FIXTURE_PUBLIC_KEY: &str = "L-V9o0fNYkMVKNqsX7spBzD_9oSvxM_C7ZCZX1jLO3Q";

    #[test]
    fn reconstructs_reality_vision_and_xhttp_from_server_json() {
        let dir = tempfile::tempdir().unwrap();
        write_xray_fixture(dir.path(), reality_fixture_with_tcp_and_xhttp());

        let load = load_xray_nodes(dir.path()).unwrap();

        assert_eq!(load.skipped, 0);
        let SubscriptionNode::VlessReality(vision) = &load.nodes[0] else {
            panic!("expected Reality node");
        };
        assert_eq!(vision.name, "fixture@example.com");
        assert_eq!(vision.port, 443);
        assert_eq!(vision.uuid, FIXTURE_UUID);
        assert_eq!(vision.server_name, "example.com");
        assert_eq!(vision.public_key, FIXTURE_PUBLIC_KEY);
        assert_eq!(vision.short_id, "0123456789abcdef");
        assert_eq!(vision.network, VlessNetwork::Tcp);
        assert_eq!(vision.flow.as_deref(), Some("xtls-rprx-vision"));
        assert!(matches!(&load.nodes[1], SubscriptionNode::VlessReality(n)
            if matches!(&n.network, VlessNetwork::Xhttp { path } if path == "/assets")));
        assert!(load.diagnostics().is_empty());
    }

    #[test]
    fn skips_bad_inbound_but_rejects_all_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        write_xray_fixture(
            dir.path(),
            fixture_with_one_valid_and_one_missing_private_key(),
        );

        let load = load_xray_nodes(dir.path()).unwrap();

        assert_eq!((load.nodes.len(), load.skipped), (1, 1));
        assert_eq!(load.diagnostics(), ["server.json inbound[1]: malformed"]);
        write_xray_fixture(dir.path(), fixture_with_only_missing_private_key());
        let invalid = load_xray_nodes(dir.path()).unwrap();
        assert_eq!(invalid.diagnostics(), ["server.json inbound[0]: malformed"]);
        let error = invalid.require_nodes().err().unwrap();
        let diagnostic = format!("{error:#}");
        assert!(!diagnostic.contains(FIXTURE_UUID));
        assert!(!diagnostic.contains(FIXTURE_PRIVATE_KEY));
    }

    #[test]
    fn reports_secret_safe_file_and_inbound_diagnostics_while_loading_valid_siblings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("00-bad.json"),
            format!(r#"{{"uuid":"{FIXTURE_UUID}","key":"{FIXTURE_PRIVATE_KEY}""#),
        )
        .unwrap();
        let mut malformed = reality_inbound("malformed", "tcp", Some("xtls-rprx-vision"), None);
        malformed["streamSettings"]["realitySettings"]
            .as_object_mut()
            .unwrap()
            .remove("privateKey");
        write_named_xray_fixture(
            dir.path(),
            "01-mixed.json",
            json!({
                "inbounds": [
                    {"protocol": "socks", "password": FIXTURE_PRIVATE_KEY},
                    malformed,
                    reality_inbound("valid", "xhttp", None, Some("/assets"))
                ]
            }),
        );

        let load = load_xray_nodes(dir.path()).unwrap();

        assert_eq!((load.nodes.len(), load.skipped), (1, 3));
        assert_eq!(
            load.diagnostics(),
            [
                "00-bad.json: malformed-json",
                "01-mixed.json inbound[0]: unsupported",
                "01-mixed.json inbound[1]: malformed",
            ]
        );
        let diagnostics = load.diagnostics().join("\n");
        assert!(!diagnostics.contains(FIXTURE_UUID));
        assert!(!diagnostics.contains(FIXTURE_PRIVATE_KEY));
    }

    #[test]
    fn loads_multiple_files_in_filename_order() {
        let dir = tempfile::tempdir().unwrap();
        write_named_xray_fixture(dir.path(), "b.json", single_reality_fixture("second"));
        write_named_xray_fixture(dir.path(), "a.json", single_reality_fixture("first"));

        let load = load_xray_nodes(dir.path()).unwrap();
        let names = load.nodes.iter().map(node_name).collect::<Vec<_>>();

        assert_eq!(names, ["first", "second"]);
        assert!(load.diagnostics().is_empty());
    }

    #[test]
    fn rescans_live_files_after_add_edit_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        write_named_xray_fixture(dir.path(), "active.json", single_reality_fixture("first"));
        assert_eq!(loaded_names(dir.path()), ["first"]);

        write_named_xray_fixture(dir.path(), "active.json", single_reality_fixture("edited"));
        assert_eq!(loaded_names(dir.path()), ["edited"]);

        write_named_xray_fixture(dir.path(), "added.json", single_reality_fixture("added"));
        assert_eq!(loaded_names(dir.path()), ["edited", "added"]);

        fs::remove_file(dir.path().join("active.json")).unwrap();
        let load = load_xray_nodes(dir.path()).unwrap();
        assert_eq!(
            load.nodes.iter().map(node_name).collect::<Vec<_>>(),
            ["added"]
        );
        assert!(load.diagnostics().is_empty());
    }

    fn write_xray_fixture(dir: &Path, fixture: Value) {
        write_named_xray_fixture(dir, "server.json", fixture);
    }

    fn write_named_xray_fixture(dir: &Path, name: &str, fixture: Value) {
        fs::write(dir.join(name), serde_json::to_vec(&fixture).unwrap()).unwrap();
    }

    fn single_reality_fixture(name: &str) -> Value {
        let mut inbound = reality_inbound(name, "tcp", Some("xtls-rprx-vision"), None);
        inbound["settings"]["clients"][0]["email"] = json!(name);
        json!({"inbounds": [inbound]})
    }

    fn node_name(node: &SubscriptionNode) -> &str {
        match node {
            SubscriptionNode::VlessReality(node) => &node.name,
            SubscriptionNode::Hysteria2(_) | SubscriptionNode::Tuic(_) => unreachable!(),
        }
    }

    fn loaded_names(dir: &Path) -> Vec<String> {
        let load = load_xray_nodes(dir).unwrap();
        assert!(load.diagnostics().is_empty());
        load.nodes
            .iter()
            .map(node_name)
            .map(str::to_owned)
            .collect()
    }

    fn reality_fixture_with_tcp_and_xhttp() -> Value {
        json!({
            "inbounds": [
                reality_inbound("reality-vision", "tcp", Some("xtls-rprx-vision"), None),
                reality_inbound("reality-xhttp", "xhttp", None, Some("/assets"))
            ]
        })
    }

    fn fixture_with_one_valid_and_one_missing_private_key() -> Value {
        let mut invalid = reality_inbound("invalid-reality", "tcp", Some("xtls-rprx-vision"), None);
        invalid["streamSettings"]["realitySettings"]
            .as_object_mut()
            .unwrap()
            .remove("privateKey");
        json!({
            "inbounds": [
                reality_inbound("valid-reality", "tcp", Some("xtls-rprx-vision"), None),
                invalid
            ]
        })
    }

    fn fixture_with_only_missing_private_key() -> Value {
        let mut fixture = fixture_with_one_valid_and_one_missing_private_key();
        fixture["inbounds"].as_array_mut().unwrap().remove(0);
        fixture
    }

    fn reality_inbound(tag: &str, network: &str, flow: Option<&str>, path: Option<&str>) -> Value {
        let mut inbound = json!({
            "listen": "0.0.0.0",
            "port": 443,
            "protocol": "vless",
            "tag": tag,
            "settings": {
                "clients": [{
                    "id": FIXTURE_UUID,
                    "email": "fixture@example.com"
                }],
                "decryption": "none"
            },
            "streamSettings": {
                "network": network,
                "security": "reality",
                "realitySettings": {
                    "target": "example.com:443",
                    "serverNames": ["example.com"],
                    "privateKey": FIXTURE_PRIVATE_KEY,
                    "shortIds": ["", "0123456789abcdef"]
                }
            }
        });
        if let Some(flow) = flow {
            inbound["settings"]["clients"][0]["flow"] = json!(flow);
        }
        if let Some(path) = path {
            inbound["streamSettings"]["xhttpSettings"] = json!({
                "host": "",
                "path": path,
                "mode": "auto"
            });
        }
        inbound
    }
}
