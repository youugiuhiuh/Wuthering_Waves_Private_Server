use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use rand::rngs::OsRng;
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use x25519_dalek::{PublicKey, StaticSecret};

const API_ENDPOINT: &str = "https://api.cloudflareclient.com/v0a2158";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WarpAccountConfig {
    pub private_key: String,
    pub public_key: String,
    pub address_v4: String,
    pub address_v6: String,
    pub reserved: Vec<u8>,
    pub client_id: String,
}

pub async fn register_account() -> Result<WarpAccountConfig> {
    // Generate keys
    let private_key = StaticSecret::random_from_rng(OsRng);
    let public_key = PublicKey::from(&private_key);

    let priv_key_b64 = general_purpose::STANDARD.encode(private_key.to_bytes());
    let pub_key_b64 = general_purpose::STANDARD.encode(public_key.as_bytes());

    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    // Register
    let reg_url = format!("{}/reg", API_ENDPOINT);
    let mut headers = header::HeaderMap::new();
    headers.insert("User-Agent", "okhttp/3.12.1".parse()?);
    headers.insert("Content-Type", "application/json; charset=UTF-8".parse()?);

    let body = json!({
        "key": pub_key_b64,
        "install_id": "",
        "fcm_token": "",
        "tos":  chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S.000+01:00").to_string(),
        "model": "PC",
        "serial_number": "",
        "locale": "en_US"
    });

    let resp = client
        .post(&reg_url)
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("WARP API 注册失败: Status {}", resp.status()));
    }

    let json: Value = resp.json().await?;

    // Parse response
    let account = &json["config"]["interface"]["addresses"];
    let v4 = account["v4"].as_str().context("No IPv4 address")?;
    let v6 = account["v6"].as_str().context("No IPv6 address")?;

    // Reserved bytes (client_id logical equivalent in config)
    // The API returns "client_id" in the config object, which is base64 encoded.
    // This MUST be used for the reserved bytes in WireGuard handshake.

    let client_id = json["config"]["client_id"]
        .as_str()
        .context("No client_id in config")?
        .to_string();

    // Cloudflare specific: reserved bytes are decoded from client_id
    let client_id_bytes = match general_purpose::STANDARD.decode(&client_id) {
        Ok(b) => b,
        Err(_) => vec![0, 0, 0], // Fallback
    };

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
