use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::Value;

pub struct TUICConfig {
    pub port: u16,
    pub uuid: String,
    pub password: String,
    pub sni: String,
    pub alpn: String,
    pub congestion_control: String,
}

impl TUICConfig {
    pub fn new(port: u16, uuid: String, password: String, sni: String) -> Self {
        Self {
            port,
            uuid,
            password,
            sni,
            alpn: "h3".to_string(),
            congestion_control: "bbr".to_string(),
        }
    }

    pub fn to_inbound_json(&self, tag: &str) -> Value {
        serde_json::json!({
            "type": "tuic",
            "tag": tag,
            "listen": "::",
            "listen_port": self.port,
            "users": [{
                "uuid": self.uuid,
                "password": self.password
            }],
            "tls": {
                "enabled": true,
                "server_name": self.sni,
                "alpn": [self.alpn],
                "certificate_path": "/etc/wwps/wwps-box/certs/tls.cer",
                "key_path": "/etc/wwps/wwps-box/certs/tls.key"
            },
            "congestion_control": self.congestion_control,
            "zero_rtt_handshake": false,
            "heartbeat": "30s"
        })
    }

    pub fn to_client_link(&self, host: &str, name: &str) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();

        format!(
            "tuic://{}:{}@{}:{}?sni={}&alpn={}&congestion_control={}&allow_insecure=1#{}",
            self.uuid,
            encoded_password,
            host,
            self.port,
            encoded_sni,
            self.alpn,
            self.congestion_control,
            encoded_name
        )
    }

    pub fn generate_password() -> String {
        let mut rng = StdRng::from_entropy();
        let chars: String = (0..16)
            .map(|_| {
                let charset = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        chars
    }
}
