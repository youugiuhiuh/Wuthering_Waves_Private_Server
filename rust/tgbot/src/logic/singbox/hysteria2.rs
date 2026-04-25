use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
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

    pub fn with_obfs(port: u16, password: String, sni: String, obfs_type: String, obfs_password: String) -> Self {
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
        map.insert("users".to_string(), serde_json::json!([
            {
                "password": self.password
            }
        ]));

        let mut tls_map = serde_json::Map::new();
        tls_map.insert("enabled".to_string(), serde_json::json!(true));
        tls_map.insert("server_name".to_string(), serde_json::json!(self.sni));
        tls_map.insert("alpn".to_string(), serde_json::json!(["h3"]));
        tls_map.insert("key_path".to_string(), serde_json::json!("/etc/wwps/wwps-box/certs/tls.key"));
        tls_map.insert("certificate_path".to_string(), serde_json::json!("/etc/wwps/wwps-box/certs/tls.cer"));
        map.insert("tls".to_string(), serde_json::json!(tls_map));

        if let Some(ref obfs_type) = self.obfs_type {
            if let Some(ref obfs_password) = self.obfs_password {
                let mut obfs_map = serde_json::Map::new();
                obfs_map.insert("type".to_string(), serde_json::json!(obfs_type));
                obfs_map.insert("password".to_string(), serde_json::json!(obfs_password));
                map.insert("obfs".to_string(), serde_json::json!(obfs_map));
            }
        }

        serde_json::Value::Object(map)
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

    pub fn to_client_link_with_hopping(&self, host: &str, name: &str, hop_range: (u16, u16)) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let hop_ports = format!("{}-{}", hop_range.0, hop_range.1);

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&insecure=1&hop_ports={}&hop_interval=30s#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            hop_ports,
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

    pub fn to_client_link_with_hopping_and_obfs(&self, host: &str, name: &str, hop_range: (u16, u16)) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let encoded_obfs_password = utf8_percent_encode(self.obfs_password.as_deref().unwrap_or(""), NON_ALPHANUMERIC).to_string();
        let hop_ports = format!("{}-{}", hop_range.0, hop_range.1);

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&insecure=1&hop_ports={}&hop_interval=30s&obfs=salamander&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            hop_ports,
            encoded_obfs_password,
            encoded_name
        )
    }
}

pub fn generate_hysteria2_password() -> String {
    Hysteria2Config::generate_password()
}