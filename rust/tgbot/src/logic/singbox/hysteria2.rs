use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::Value;

pub struct Hysteria2Config {
    pub port: u16,
    pub password: String,
    pub sni: String,
    pub obfs_type: Option<String>,
    pub obfs_password: Option<String>,
}

impl Hysteria2Config {
    pub fn new(port: u16, password: String, sni: String) -> Self {
        Self {
            port,
            password,
            sni,
            obfs_type: None,
            obfs_password: None,
        }
    }

    pub fn with_obfs(
        port: u16,
        password: String,
        sni: String,
        obfs_type: String,
        obfs_password: String,
    ) -> Self {
        Self {
            port,
            password,
            sni,
            obfs_type: Some(obfs_type),
            obfs_password: Some(obfs_password),
        }
    }

    pub fn to_inbound_json(&self, tag: &str) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("type".to_string(), serde_json::json!("hysteria2"));
        map.insert("tag".to_string(), serde_json::json!(tag));
        map.insert("listen".to_string(), serde_json::json!("::"));
        map.insert("listen_port".to_string(), serde_json::json!(self.port));
        map.insert(
            "users".to_string(),
            serde_json::json!([
                {
                    "password": self.password
                }
            ]),
        );

        let mut tls_map = serde_json::Map::new();
        tls_map.insert("enabled".to_string(), serde_json::json!(true));
        tls_map.insert("server_name".to_string(), serde_json::json!(self.sni));
        tls_map.insert("alpn".to_string(), serde_json::json!(["h3"]));
        tls_map.insert(
            "key_path".to_string(),
            serde_json::json!("/etc/wwps/wwps-box/certs/tls.key"),
        );
        tls_map.insert(
            "certificate_path".to_string(),
            serde_json::json!("/etc/wwps/wwps-box/certs/tls.cer"),
        );
        map.insert("tls".to_string(), serde_json::json!(tls_map));

        if let Some(ref obfs_type) = self.obfs_type
            && let Some(ref obfs_password) = self.obfs_password
        {
            let mut obfs_map = serde_json::Map::new();
            obfs_map.insert("type".to_string(), serde_json::json!(obfs_type));
            obfs_map.insert("password".to_string(), serde_json::json!(obfs_password));
            map.insert("obfs".to_string(), serde_json::json!(obfs_map));
        }

        serde_json::Value::Object(map)
    }

    pub fn to_client_link(&self, host: &str, name: &str) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&insecure=1#{}",
            encoded_password, host, self.port, encoded_sni, encoded_name
        )
    }

    pub fn to_client_link_with_hopping(
        &self,
        host: &str,
        name: &str,
        hop_range: (u16, u16),
    ) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();

        format!(
            "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3&insecure=1&hop_interval=30s#{}",
            encoded_password, host, self.port, hop_range.0, hop_range.1, encoded_sni, encoded_name
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

    pub fn generate_obfs_password() -> String {
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

    pub fn to_client_link_with_hopping_and_obfs(
        &self,
        host: &str,
        name: &str,
        hop_range: (u16, u16),
    ) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let encoded_obfs_password = utf8_percent_encode(
            self.obfs_password.as_deref().unwrap_or(""),
            NON_ALPHANUMERIC,
        )
        .to_string();

        format!(
            "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3&insecure=1&hop_interval=30s&obfs=salamander&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            hop_range.0,
            hop_range.1,
            encoded_sni,
            encoded_obfs_password,
            encoded_name
        )
    }
}

pub fn generate_hysteria2_password() -> String {
    Hysteria2Config::generate_password()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hysteria2_config_new() {
        let config = Hysteria2Config::new(8443, "test_password".to_string(), "sni.example.com".to_string());
        assert_eq!(config.port, 8443);
        assert_eq!(config.password, "test_password");
        assert_eq!(config.sni, "sni.example.com");
        assert!(config.obfs_type.is_none());
        assert!(config.obfs_password.is_none());
    }

    #[test]
    fn test_hysteria2_config_with_obfs() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
            "salamander".to_string(),
            "obfs_secret".to_string(),
        );
        assert_eq!(config.port, 8443);
        assert_eq!(config.obfs_type, Some("salamander".to_string()));
        assert_eq!(config.obfs_password, Some("obfs_secret".to_string()));
    }

    #[test]
    fn test_hysteria2_to_inbound_json_basic() {
        let config = Hysteria2Config::new(8443, "pw".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert_eq!(json["type"], "hysteria2");
        assert_eq!(json["tag"], "test-tag");
        assert_eq!(json["listen"], "::");
        assert_eq!(json["listen_port"], 8443);
        assert_eq!(json["users"][0]["password"], "pw");
        assert!(json["tls"]["enabled"].as_bool().unwrap());
        assert_eq!(json["tls"]["server_name"], "sni.example.com");
    }

    #[test]
    fn test_hysteria2_to_inbound_json_with_obfs() {
        let config = Hysteria2Config::with_obfs(8443, "pw".to_string(), "sni.example.com".to_string(), "salamander".to_string(), "obfs123".to_string());
        let json = config.to_inbound_json("test-tag");
        assert!(json["obfs"].is_object());
        assert_eq!(json["obfs"]["type"], "salamander");
        assert_eq!(json["obfs"]["password"], "obfs123");
    }

    #[test]
    fn test_hysteria2_to_inbound_json_tls_fields() {
        let config = Hysteria2Config::new(8443, "pw".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert_eq!(json["tls"]["key_path"], "/etc/wwps/wwps-box/certs/tls.key");
        assert_eq!(json["tls"]["certificate_path"], "/etc/wwps/wwps-box/certs/tls.cer");
        assert_eq!(json["tls"]["alpn"], serde_json::json!(["h3"]));
    }

    #[test]
    fn test_hysteria2_to_client_link_basic() {
        let config = Hysteria2Config::new(8443, "mypassword".to_string(), "sni.example.com".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("@1.2.3.4:8443"));
        assert!(link.contains("sni="));
        assert!(link.contains("#MyNode"));
    }

    #[test]
    fn test_hysteria2_to_client_link_encoding() {
        let config = Hysteria2Config::new(8443, "p@ss!word".to_string(), "sni.example.com".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.contains("p%40ss%21word"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_hopping() {
        let config = Hysteria2Config::new(8443, "mypassword".to_string(), "sni.example.com".to_string());
        let link = config.to_client_link_with_hopping("1.2.3.4", "MyNode", (8444, 8543));
        assert!(link.contains("8444-8543"));
        assert!(link.contains("hop_interval=30s"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_hopping_and_obfs() {
        let config = Hysteria2Config::with_obfs(8443, "mypassword".to_string(), "sni.example.com".to_string(), "salamander".to_string(), "obfs123".to_string());
        let link = config.to_client_link_with_hopping_and_obfs("1.2.3.4", "MyNode", (8444, 8543));
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(link.contains("hop_interval=30s"));
    }

    #[test]
    fn test_hysteria2_generate_password_length() {
        let pw = Hysteria2Config::generate_password();
        assert_eq!(pw.len(), 32);
    }

    #[test]
    fn test_hysteria2_generate_password_charset() {
        let pw = Hysteria2Config::generate_password();
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_hysteria2_generate_password_uniqueness() {
        let pw1 = Hysteria2Config::generate_password();
        let pw2 = Hysteria2Config::generate_password();
        assert_ne!(pw1, pw2);
    }

    #[test]
    fn test_generate_hysteria2_password() {
        let pw = generate_hysteria2_password();
        assert_eq!(pw.len(), 32);
    }
}
