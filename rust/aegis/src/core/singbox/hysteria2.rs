use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::Value;

/// Hysteria2 QUIC traffic obfuscation type (`obfs.type`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hysteria2ObfsType {
    Salamander,
    Gecko,
}

impl Hysteria2ObfsType {
    /// JSON and client URI value for this obfuscation type.
    pub fn as_str(self) -> &'static str {
        match self {
            Hysteria2ObfsType::Salamander => "salamander",
            Hysteria2ObfsType::Gecko => "gecko",
        }
    }
}

/// Default minimum on-wire packet size in bytes for gecko (sing-box default).
pub const GECKO_DEFAULT_MIN_PACKET_SIZE: usize = 512;

/// Default maximum on-wire packet size in bytes for gecko (sing-box default).
pub const GECKO_DEFAULT_MAX_PACKET_SIZE: usize = 1200;

/// 端口跳跃分享链接的目标客户端格式。
/// - `Official`: 官方 URI Scheme 的端口位置 multi-port + `hop_interval`
/// - `V2rayN`: v2rayN 系客户端的 `mport` 查询参数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hy2LinkStyle {
    Official,
    V2rayN,
}

pub struct Hysteria2Config {
    pub port: u16,
    pub password: String,
    pub sni: String,
    pub obfs_type: Option<Hysteria2ObfsType>,
    pub obfs_password: Option<String>,
    pub pin_sha256: Option<String>,
}

impl Hysteria2Config {
    pub fn new(port: u16, password: String, sni: String) -> Self {
        Self {
            port,
            password,
            sni,
            obfs_type: None,
            obfs_password: None,
            pin_sha256: None,
        }
    }

    pub fn with_obfs(
        port: u16,
        password: String,
        sni: String,
        obfs_type: Hysteria2ObfsType,
        obfs_password: String,
    ) -> Self {
        Self {
            port,
            password,
            sni,
            obfs_type: Some(obfs_type),
            obfs_password: Some(obfs_password),
            pin_sha256: None,
        }
    }

    pub fn with_pin_sha256(mut self, pin: String) -> Self {
        self.pin_sha256 = Some(pin);
        self
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
            obfs_map.insert("type".to_string(), serde_json::json!(obfs_type.as_str()));
            obfs_map.insert("password".to_string(), serde_json::json!(obfs_password));
            if *obfs_type == Hysteria2ObfsType::Gecko {
                obfs_map.insert(
                    "min_packet_size".to_string(),
                    serde_json::json!(GECKO_DEFAULT_MIN_PACKET_SIZE),
                );
                obfs_map.insert(
                    "max_packet_size".to_string(),
                    serde_json::json!(GECKO_DEFAULT_MAX_PACKET_SIZE),
                );
            }
            map.insert("obfs".to_string(), serde_json::json!(obfs_map));
        }

        serde_json::Value::Object(map)
    }

    pub fn to_client_link(&self, host: &str, name: &str) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let pin_param = self
            .pin_sha256
            .as_ref()
            .map(|p| format!("&pinSHA256={}", p))
            .unwrap_or_default();

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3{}#{}",
            encoded_password, host, self.port, encoded_sni, pin_param, encoded_name
        )
    }

    pub fn to_client_link_with_obfs(&self, host: &str, name: &str) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let encoded_obfs_password = utf8_percent_encode(
            self.obfs_password.as_deref().unwrap_or(""),
            NON_ALPHANUMERIC,
        )
        .to_string();
        let obfs_value = self.obfs_type.map(|t| t.as_str()).unwrap_or("salamander");
        let pin_param = self
            .pin_sha256
            .as_ref()
            .map(|p| format!("&pinSHA256={}", p))
            .unwrap_or_default();

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&obfs={}&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            pin_param,
            obfs_value,
            encoded_obfs_password,
            encoded_name
        )
    }

    pub fn to_client_link_with_hopping(
        &self,
        host: &str,
        name: &str,
        hop_range: (u16, u16),
        style: Hy2LinkStyle,
    ) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let pin_param = self
            .pin_sha256
            .as_ref()
            .map(|p| format!("&pinSHA256={}", p))
            .unwrap_or_default();
        match style {
            Hy2LinkStyle::Official => format!(
                "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3{}&hop_interval=30s#{}",
                encoded_password,
                host,
                self.port,
                hop_range.0,
                hop_range.1,
                encoded_sni,
                pin_param,
                encoded_name
            ),
            Hy2LinkStyle::V2rayN => format!(
                "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&mport={}-{}#{}",
                encoded_password,
                host,
                self.port,
                encoded_sni,
                pin_param,
                hop_range.0,
                hop_range.1,
                encoded_name
            ),
        }
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
        style: Hy2LinkStyle,
    ) -> String {
        let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
        let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let encoded_obfs_password = utf8_percent_encode(
            self.obfs_password.as_deref().unwrap_or(""),
            NON_ALPHANUMERIC,
        )
        .to_string();
        let obfs_value = self.obfs_type.map(|t| t.as_str()).unwrap_or("salamander");
        let pin_param = self
            .pin_sha256
            .as_ref()
            .map(|p| format!("&pinSHA256={}", p))
            .unwrap_or_default();
        match style {
            Hy2LinkStyle::Official => format!(
                "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3{}&hop_interval=30s&obfs={}&obfs-password={}#{}",
                encoded_password,
                host,
                self.port,
                hop_range.0,
                hop_range.1,
                encoded_sni,
                pin_param,
                obfs_value,
                encoded_obfs_password,
                encoded_name
            ),
            Hy2LinkStyle::V2rayN => format!(
                "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&mport={}-{}&obfs={}&obfs-password={}#{}",
                encoded_password,
                host,
                self.port,
                encoded_sni,
                pin_param,
                hop_range.0,
                hop_range.1,
                obfs_value,
                encoded_obfs_password,
                encoded_name
            ),
        }
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
        let config = Hysteria2Config::new(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
        );
        assert_eq!(config.port, 8443);
        assert_eq!(config.password, "test_password");
        assert_eq!(config.sni, "sni.example.com");
        assert!(config.obfs_type.is_none());
        assert!(config.obfs_password.is_none());
        assert!(config.pin_sha256.is_none());
    }

    #[test]
    fn test_hysteria2_config_with_obfs() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs_secret".to_string(),
        );
        assert_eq!(config.port, 8443);
        assert_eq!(config.obfs_type, Some(Hysteria2ObfsType::Salamander));
        assert_eq!(config.obfs_password, Some("obfs_secret".to_string()));
        assert!(config.pin_sha256.is_none());
    }

    #[test]
    fn test_obfs_type_as_str() {
        assert_eq!(Hysteria2ObfsType::Salamander.as_str(), "salamander");
        assert_eq!(Hysteria2ObfsType::Gecko.as_str(), "gecko");
    }

    #[test]
    fn test_obfs_type_copy() {
        let t = Hysteria2ObfsType::Gecko;
        let t2 = t; // Copy, not move
        assert_eq!(t, t2);
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
        let config = Hysteria2Config::with_obfs(
            8443,
            "pw".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs123".to_string(),
        );
        let json = config.to_inbound_json("test-tag");
        assert!(json["obfs"].is_object());
        assert_eq!(json["obfs"]["type"], "salamander");
        assert_eq!(json["obfs"]["password"], "obfs123");
        assert!(json["obfs"].get("min_packet_size").is_none());
        assert!(json["obfs"].get("max_packet_size").is_none());
    }

    #[test]
    fn test_hysteria2_to_inbound_json_with_gecko() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "pw".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        );
        let json = config.to_inbound_json("test-tag");
        assert!(json["obfs"].is_object());
        assert_eq!(json["obfs"]["type"], "gecko");
        assert_eq!(json["obfs"]["password"], "obfs123");
        assert_eq!(json["obfs"]["min_packet_size"], 512);
        assert_eq!(json["obfs"]["max_packet_size"], 1200);
    }

    #[test]
    fn test_hysteria2_to_inbound_json_tls_fields() {
        let config = Hysteria2Config::new(8443, "pw".to_string(), "sni.example.com".to_string());
        let json = config.to_inbound_json("test-tag");
        assert_eq!(json["tls"]["key_path"], "/etc/wwps/wwps-box/certs/tls.key");
        assert_eq!(
            json["tls"]["certificate_path"],
            "/etc/wwps/wwps-box/certs/tls.cer"
        );
        assert_eq!(json["tls"]["alpn"], serde_json::json!(["h3"]));
    }

    #[test]
    fn test_hysteria2_to_client_link_basic() {
        let config = Hysteria2Config::new(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("@1.2.3.4:8443"));
        assert!(link.contains("sni="));
        assert!(link.contains("pinSHA256=AA:BB:CC"));
        assert!(link.contains("#MyNode"));
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_to_client_link_encoding() {
        let config =
            Hysteria2Config::new(8443, "p@ss!word".to_string(), "sni.example.com".to_string())
                .with_pin_sha256("AA:BB".to_string());
        let link = config.to_client_link("1.2.3.4", "MyNode");
        assert!(link.contains("p%40ss%21word"));
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_hopping() {
        let config = Hysteria2Config::new(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_hopping(
            "1.2.3.4",
            "MyNode",
            (8444, 8543),
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("pinSHA256=AA:BB:CC"));
        assert!(link.contains("8444-8543"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_obfs_no_hopping() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_obfs("1.2.3.4", "MyNode");
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(link.contains("pinSHA256=AA:BB:CC"));
        assert!(!link.contains("insecure=1"));
        assert!(!link.contains("hop_interval=30s"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_gecko_no_hopping() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_obfs("1.2.3.4", "MyNode");
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(!link.contains("obfs=salamander"));
        assert!(!link.contains("hop_interval=30s"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_gecko_hopping() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_hopping_and_obfs(
            "1.2.3.4",
            "MyNode",
            (8444, 8543),
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("obfs=salamander"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_hopping_and_obfs() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_hopping_and_obfs(
            "1.2.3.4",
            "MyNode",
            (8444, 8543),
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("pinSHA256=AA:BB:CC"));
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_to_client_link_hopping_v2rayn_style() {
        let config = Hysteria2Config::new(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
        );
        let link = config.to_client_link_with_hopping(
            "1.2.3.4",
            "MyNode",
            (8444, 8543),
            Hy2LinkStyle::V2rayN,
        );
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("@1.2.3.4:8443?"));
        assert!(!link.contains(":8443,8444"));
        assert!(link.contains("mport=8444-8543"));
        assert!(!link.contains("hop_interval"));
        assert!(link.contains("sni="));
        assert!(link.contains("#MyNode"));
    }

    #[test]
    fn test_hysteria2_to_client_link_hopping_obfs_v2rayn_style() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs_secret".to_string(),
        );
        let link = config.to_client_link_with_hopping_and_obfs(
            "1.2.3.4",
            "MyNode",
            (8444, 8543),
            Hy2LinkStyle::V2rayN,
        );
        assert!(link.contains("@1.2.3.4:8443?"));
        assert!(link.contains("mport=8444-8543"));
        assert!(!link.contains("hop_interval"));
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password="));
    }

    #[test]
    fn test_hysteria2_to_client_link_hopping_v2rayn_keeps_pin() {
        let config = Hysteria2Config::new(8443, "pw".to_string(), "s.example.com".to_string())
            .with_pin_sha256("AA:BB:CC".to_string());
        let link =
            config.to_client_link_with_hopping("1.2.3.4", "N", (8444, 8543), Hy2LinkStyle::V2rayN);
        assert!(link.contains("pinSHA256=AA:BB:CC"));
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
