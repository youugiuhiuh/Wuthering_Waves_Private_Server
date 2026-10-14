use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use obfstr::obfstr;
use rand::RngExt as _;
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::core::xray::WarpScope;

// const API_ENDPOINT moved into register_account

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WarpAccountConfig {
    pub private_key: String,
    pub public_key: String,
    pub address_v4: String,
    pub address_v6: String,
    pub reserved: Vec<u8>,
    pub client_id: String,

    /// WARP 的流量范围（谁走 WARP 出口）。
    ///
    /// `#[serde(default)]`：既有的 `warp_account.json` 没有这个字段，
    /// 反序列化时必须回到 [`WarpScope::Domain`]（现有行为），否则升级后
    /// 会让既有部署「静默开启全局 WARP」。
    ///
    /// 选在 `warp_account.json` 而非 Xray 配置文件落盘：Xray 从不读这个
    /// 文件，写入零侵入，也不会因未知顶层键导致核心启动失败。
    #[serde(default)]
    pub scope: WarpScope,
}

pub async fn register_account() -> Result<WarpAccountConfig> {
    // Generate keys
    let private_key = StaticSecret::random();
    let public_key = PublicKey::from(&private_key);

    let priv_key_b64 = general_purpose::STANDARD.encode(private_key.to_bytes());
    let pub_key_b64 = general_purpose::STANDARD.encode(public_key.as_bytes());

    crate::bootstrap::install_crypto_provider();
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    let api_endpoint = obfstr!("https://api.cloudflareclient.com/v0a2158").to_string();
    let reg_path = obfstr!("/reg").to_string();
    let ua_str = obfstr!("2024.2.62.0").to_string();
    let ct_str = obfstr!("application/json; charset=UTF-8").to_string();
    let model_str = obfstr!("PC").to_string();
    let locale_str = obfstr!("en_US").to_string();
    let err_prefix = obfstr!("WARP API 注册失败").to_string();

    // Register
    let reg_url = format!("{}{}", api_endpoint, reg_path);
    let mut headers = header::HeaderMap::new();
    // Emulate official client UA to potentially get better IP reputation
    if let Ok(ua) = header::HeaderValue::from_str(&ua_str) {
        headers.insert("User-Agent", ua);
    }
    if let Ok(ct) = header::HeaderValue::from_str(&ct_str) {
        headers.insert("Content-Type", ct);
    }

    // Generate random install_id (22 chars)
    let install_id_str: String = std::iter::repeat_with(|| {
        let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let idx = rand::rng().random_range(0..charset.len());
        charset[idx] as char
    })
    .take(22)
    .collect();

    let body = json!({
        "key": pub_key_b64,
        "install_id": install_id_str,
        "fcm_token": "",
        "tos": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "model": model_str,
        "serial_number": "",
        "locale": locale_str
    });

    let resp = client
        .post(&reg_url)
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}: Status {} - {}", err_prefix, status, text));
    }

    let json: Value = resp.json().await?;

    // Parse response
    let account = &json["config"]["interface"]["addresses"];
    let v4 = account["v4"].as_str().context("No IPv4 address")?;
    let v6 = account["v6"].as_str().context("No IPv6 address")?;

    // Reserved bytes logic (Critical for handshake)
    let client_id = json["config"]["client_id"]
        .as_str()
        .context("No client_id in config")?
        .to_string();

    // specific: reserved bytes are decoded from client_id (base64)
    let client_id_bytes = general_purpose::STANDARD
        .decode(&client_id)
        .context("Failed to decode client_id base64")?;

    // The reserved field in wireguard config is usually 3 bytes.
    let reserved = if client_id_bytes.len() >= 3 {
        client_id_bytes[0..3].to_vec()
    } else {
        vec![0, 0, 0]
    };

    Ok(WarpAccountConfig {
        private_key: priv_key_b64,
        public_key: pub_key_b64,
        address_v4: v4.to_string(),
        address_v6: v6.to_string(),
        reserved,
        client_id,
        // 新注册的账号默认「仅清单内域名走 WARP」——与既有行为一致。
        scope: WarpScope::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warp_account_config_serialization() {
        let config = WarpAccountConfig {
            private_key: "test_private_key".to_string(),
            public_key: "test_public_key".to_string(),
            address_v4: "172.16.0.1/32".to_string(),
            address_v6: "2606:4700:::1/128".to_string(),
            reserved: vec![0, 0, 0],
            client_id: "test_client_id".to_string(),
            scope: WarpScope::Domain,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("test_private_key"));
        assert!(json.contains("test_public_key"));
    }

    #[test]
    fn test_warp_account_config_deserialization() {
        let json = r#"{
            "private_key": "abc123",
            "public_key": "def456",
            "address_v4": "172.16.0.1/32",
            "address_v6": "2606:4700:::1/128",
            "reserved": [0, 0, 0],
            "client_id": "xyz789"
        }"#;

        let config: WarpAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.private_key, "abc123");
        assert_eq!(config.public_key, "def456");
        assert_eq!(config.address_v4, "172.16.0.1/32");
    }

    #[test]
    fn test_warp_account_config_default_reserved() {
        let config = WarpAccountConfig {
            private_key: "key".to_string(),
            public_key: "pub".to_string(),
            address_v4: "1.2.3.4/32".to_string(),
            address_v6: "::1/128".to_string(),
            reserved: vec![],
            client_id: "id".to_string(),
            scope: WarpScope::Domain,
        };

        assert!(config.reserved.is_empty());
    }

    /// 向后兼容硬约束：既有的 `warp_account.json` **没有** `scope` 字段。
    ///
    /// 真机上该文件的真实形状（脱敏后）为：
    /// ```json
    /// {"private_key":"…","public_key":"…","address_v4":"172.16.0.2",
    ///  "address_v6":"2606:4700:110:…::…","reserved":[164,217,153],"client_id":"…"}
    /// ```
    /// 升级后必须回退到 `Domain`。若默认为 `Global`，升级会把用户
    /// 静默切到「全部流量走 WARP」——在本部署（出站 UDP 被封）下即全体断网。
    #[test]
    fn test_scope_defaults_when_absent_legacy_file() {
        let legacy = r#"{
            "private_key": "k",
            "public_key": "p",
            "address_v4": "172.16.0.2",
            "address_v6": "2606:4700:110::1",
            "reserved": [164, 217, 153],
            "client_id": "cid"
        }"#;

        let cfg: WarpAccountConfig =
            serde_json::from_str(legacy).expect("无 scope 字段的旧文件必须仍可反序列化");
        assert_eq!(cfg.scope, WarpScope::Domain);
        assert!(!cfg.scope.is_global());
    }

    /// 新格式往返保真。
    #[test]
    fn test_scope_round_trips() {
        for scope in [WarpScope::Domain, WarpScope::Global] {
            let cfg = WarpAccountConfig {
                private_key: "k".to_string(),
                public_key: "p".to_string(),
                address_v4: "172.16.0.2".to_string(),
                address_v6: "2606:4700:110::1".to_string(),
                reserved: vec![1, 2, 3],
                client_id: "cid".to_string(),
                scope,
            };
            let json = serde_json::to_string(&cfg).unwrap();
            let back: WarpAccountConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(back.scope, scope, "scope 往返后应不变");
        }
    }

    /// 写入后的 JSON 必须以 snake_case 出现 `"scope"` 键。
    #[test]
    fn test_scope_is_persisted_under_snake_case_key() {
        let cfg = WarpAccountConfig {
            private_key: "k".to_string(),
            public_key: "p".to_string(),
            address_v4: "1.2.3.4".to_string(),
            address_v6: "::1".to_string(),
            reserved: vec![],
            client_id: "id".to_string(),
            scope: WarpScope::Global,
        };
        let v: Value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["scope"], json!("global"));
    }
}
