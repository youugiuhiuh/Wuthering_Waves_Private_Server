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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tuic_config_new_defaults() {
        let config = TUICConfig::new(
            9443,
            "uuid-1234".to_string(),
            "password".to_string(),
            "sni.example.com".to_string(),
        );
        assert_eq!(config.port, 9443);
        assert_eq!(config.uuid, "uuid-1234");
        assert_eq!(config.alpn, "h3");
        assert_eq!(config.congestion_control, "bbr");
    }

    #[test]
    fn test_tuic_to_inbound_json_structure() {
        let config = TUICConfig::new(9443, "uuid-1234".to_string(), "password".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert_eq!(json["type"], "tuic");
        assert_eq!(json["tag"], "test-tag");
        assert_eq!(json["listen"], "::");
        assert_eq!(json["listen_port"], 9443);
        assert_eq!(json["users"][0]["uuid"], "uuid-1234");
        assert_eq!(json["users"][0]["password"], "password");
    }

    #[test]
    fn test_tuic_to_inbound_json_tls_fields() {
        let config = TUICConfig::new(9443, "uuid-1234".to_string(), "password".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert!(json["tls"]["enabled"].as_bool().unwrap());
        assert_eq!(json["tls"]["server_name"], "sni.example.com");
        assert_eq!(json["tls"]["alpn"], serde_json::json!(["h3"]));
        assert_eq!(json["tls"]["key_path"], "/etc/wwps/wwps-box/certs/tls.key");
        assert_eq!(json["tls"]["certificate_path"], "/etc/wwps/wwps-box/certs/tls.cer");
    }

    #[test]
    fn test_tuic_to_inbound_json_heartbeat_zero_rtt() {
        let config = TUICConfig::new(9443, "uuid-1234".to_string(), "password".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert_eq!(json["heartbeat"], "30s");
        assert_eq!(json["zero_rtt_handshake"], false);
        assert_eq!(json["congestion_control"], "bbr");
    }

    #[test]
    fn test_tuic_to_client_link_format() {
        let config = TUICConfig::new(9443, "uuid-1234".to_string(), "password".to_string(), "sni.example.com".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.starts_with("tuic://"));
        assert!(link.contains("uuid-1234:password@"));
        assert!(link.contains("@1.2.3.4:9443"));
        assert!(link.contains("sni="));
        assert!(link.contains("congestion_control=bbr"));
        assert!(link.contains("#MyNode"));
    }

    #[test]
    fn test_tuic_to_client_link_encoding() {
        let config = TUICConfig::new(9443, "uuid-1234".to_string(), "p@ss!word".to_string(), "sni.example.com".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.contains("p%40ss%21word"));
    }

    #[test]
    fn test_tuic_generate_password_length() {
        let pw = TUICConfig::generate_password();
        assert_eq!(pw.len(), 16);
    }

    #[test]
    fn test_tuic_generate_password_charset() {
        let pw = TUICConfig::generate_password();
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_tuic_generate_password_uniqueness() {
        let pw1 = TUICConfig::generate_password();
        let pw2 = TUICConfig::generate_password();
        assert_ne!(pw1, pw2);
    }
}
