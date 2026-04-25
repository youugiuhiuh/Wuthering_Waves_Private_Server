use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::Value;

pub struct Hysteria2Config {
    pub port: u16,
    pub password: String,
    pub sni: String,
}

impl Hysteria2Config {
    pub fn new(port: u16, password: String, sni: String) -> Self {
        Self {
            port,
            password,
            sni,
        }
    }

    pub fn to_inbound_json(&self, tag: &str) -> Value {
        serde_json::json!({
            "type": "hysteria2",
            "tag": tag,
            "listen": "::",
            "listen_port": self.port,
            "users": [
                {
                    "password": self.password
                }
            ],
            "tls": {
                "enabled": true,
                "server_name": self.sni,
                "alpn": ["h3"],
                "key_path": "/etc/wwps/wwps-box/certs/tls.key",
                "certificate_path": "/etc/wwps/wwps-box/certs/tls.cer"
            }
        })
    }

    pub fn to_client_link(&self, host: &str, name: &str) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&insecure=1#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            encoded_name
        )
    }

    pub fn generate_password() -> String {
        let mut rng = StdRng::from_entropy();
        let chars: String = (0..32)
            .map(|_| {
                let charset = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        chars
    }
}

pub fn generate_hysteria2_password() -> String {
    Hysteria2Config::generate_password()
}