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
}

impl NodeLoad {
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
    };
    for path in paths {
        let Ok(raw) = fs::read(&path) else {
            load.skipped += 1;
            continue;
        };
        let Ok(config) = serde_json::from_slice::<Value>(&raw) else {
            load.skipped += 1;
            continue;
        };
        let Some(inbounds) = config.get("inbounds").and_then(Value::as_array) else {
            load.skipped += 1;
            continue;
        };
        for inbound in inbounds {
            if let Some(node) = parse_reality_inbound(inbound) {
                load.nodes.push(SubscriptionNode::VlessReality(node));
            } else {
                load.skipped += 1;
            }
        }
    }
    Ok(load)
}

fn parse_reality_inbound(inbound: &Value) -> Option<VlessRealityNode> {
    if inbound.get("protocol")?.as_str()? != "vless" {
        return None;
    }
    let stream = inbound.get("streamSettings")?;
    if stream.get("security")?.as_str()? != "reality" {
        return None;
    }
    let client = inbound
        .get("settings")?
        .get("clients")?
        .as_array()?
        .first()?;
    let flow = client
        .get("flow")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let network = match stream.get("network")?.as_str()? {
        "tcp" if flow.as_deref() == Some("xtls-rprx-vision") => VlessNetwork::Tcp,
        "xhttp" if flow.is_none() => VlessNetwork::Xhttp {
            path: stream
                .get("xhttpSettings")?
                .get("path")?
                .as_str()?
                .to_owned(),
        },
        _ => return None,
    };
    let reality = stream.get("realitySettings")?;
    let private_key = URL_SAFE_NO_PAD
        .decode(reality.get("privateKey")?.as_str()?)
        .ok()?;
    let private_key: [u8; 32] = private_key.try_into().ok()?;
    let public_key = PublicKey::from(&StaticSecret::from(private_key));

    Some(VlessRealityNode {
        name: client.get("email")?.as_str()?.to_owned(),
        port: inbound.get("port")?.as_u64()?.try_into().ok()?,
        uuid: client.get("id")?.as_str()?.to_owned(),
        server_name: reality
            .get("serverNames")?
            .as_array()?
            .first()?
            .as_str()?
            .to_owned(),
        public_key: URL_SAFE_NO_PAD.encode(public_key.as_bytes()),
        short_id: reality
            .get("shortIds")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .find(|id| !id.is_empty())?
            .to_owned(),
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

    #[test]
    fn reconstructs_reality_vision_and_xhttp_from_server_json() {
        let dir = tempfile::tempdir().unwrap();
        write_xray_fixture(dir.path(), reality_fixture_with_tcp_and_xhttp());

        let load = load_xray_nodes(dir.path()).unwrap();

        assert_eq!(load.skipped, 0);
        assert!(matches!(&load.nodes[0], SubscriptionNode::VlessReality(n)
            if n.network == VlessNetwork::Tcp
                && n.flow.as_deref() == Some("xtls-rprx-vision")));
        assert!(matches!(&load.nodes[1], SubscriptionNode::VlessReality(n)
            if matches!(&n.network, VlessNetwork::Xhttp { path } if path == "/assets")));
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
        write_xray_fixture(dir.path(), fixture_with_only_missing_private_key());
        let error = load_xray_nodes(dir.path())
            .unwrap()
            .require_nodes()
            .err()
            .unwrap();
        let diagnostic = format!("{error:#}");
        assert!(!diagnostic.contains(FIXTURE_UUID));
        assert!(!diagnostic.contains(FIXTURE_PRIVATE_KEY));
    }

    fn write_xray_fixture(dir: &Path, fixture: Value) {
        fs::write(
            dir.join("server.json"),
            serde_json::to_vec(&fixture).unwrap(),
        )
        .unwrap();
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
