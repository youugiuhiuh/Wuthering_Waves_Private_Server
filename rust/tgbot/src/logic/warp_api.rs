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
    // Emulate official client UA to potentially get better IP reputation
    headers.insert("User-Agent", "2024.2.62.0".parse()?);
    headers.insert("Content-Type", "application/json; charset=UTF-8".parse()?);

    // Generate random install_id (22 chars)
    let install_id: String = std::iter::repeat_with(|| {
        let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let idx = rand::Rng::gen_range(&mut OsRng, 0..charset.len());
        charset[idx] as char
    })
    .take(22)
    .collect();

    let body = json!({
        "key": pub_key_b64,
        "install_id": install_id,
        "fcm_token": "",
        "tos": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("WARP API 注册失败: Status {} - {}", status, text));
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
