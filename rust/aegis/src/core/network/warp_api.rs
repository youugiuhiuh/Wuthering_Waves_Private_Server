use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use obfstr::obfstr;
use rand::rngs::OsRng;
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use x25519_dalek::{PublicKey, StaticSecret};

// const API_ENDPOINT moved into register_account

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WarpAccountConfig {
    pub private_key: String,
    pub public_key: String,
    pub address_v4: String,
    pub address_v6: String,
    pub reserved: Vec<u8>,
    pub client_id: String,
}

/// Register a new WARP account with the Cloudflare API.
///
/// # Errors
///
/// Returns an error if the HTTP request to the Cloudflare API fails or
/// returns a non-success status, or if the response JSON is malformed.
pub async fn register_account() -> Result<WarpAccountConfig> {
    // Generate keys
    let private_key = StaticSecret::random_from_rng(OsRng);
    let public_key = PublicKey::from(&private_key);

    let priv_key_b64 = general_purpose::STANDARD.encode(private_key.to_bytes());
    let pub_key_b64 = general_purpose::STANDARD.encode(public_key.as_bytes());

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
        let idx = rand::Rng::gen_range(&mut OsRng, 0..charset.len());
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
        };

        assert!(config.reserved.is_empty());
    }
}
