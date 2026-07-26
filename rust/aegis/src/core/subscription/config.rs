use std::{
    fs::{self, File},
    io,
    net::IpAddr,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::core::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateMode {
    Domain,
    Ip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    pub enabled: bool,
    pub port: u16,
    pub public_host: String,
    pub ipv6_san: Option<IpAddr>,
    pub token_hash: String,
    pub certificate_mode: CertificateMode,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

impl SubscriptionConfig {
    pub fn new_disabled(token_hash: impl Into<String>) -> Self {
        Self {
            enabled: false,
            port: 443,
            public_host: String::new(),
            ipv6_san: None,
            token_hash: token_hash.into(),
            certificate_mode: CertificateMode::Domain,
            cert_path: paths::subscription::LIVE_CERT.into(),
            key_path: paths::subscription::LIVE_KEY.into(),
        }
    }

    pub fn load_from(path: &Path) -> Result<Option<Self>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        let temporary_path = path.with_extension("json.tmp");
        let file = File::create(&temporary_path)?;
        serde_json::to_writer(&file, self)?;
        file.sync_all()?;
        fs::rename(temporary_path, path)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.token_hash.len() != 64
            || !self.token_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("token hash must be a 64-character hexadecimal digest");
        }

        let host = normalize_host(&self.public_host);
        if host.is_empty() {
            bail!("public host must not be empty");
        }
        if self.port == 80 {
            bail!("port 80 is reserved for ACME HTTP-01");
        }

        let parsed_host = host.parse::<IpAddr>();
        match self.certificate_mode {
            CertificateMode::Domain if parsed_host.is_ok() => {
                bail!("domain certificate mode requires a domain host");
            }
            CertificateMode::Ip if parsed_host.is_err() => {
                bail!("IP certificate mode requires an IP host");
            }
            _ => {}
        }

        if self.ipv6_san.is_some_and(|address| !address.is_ipv6()) {
            bail!("optional SAN must be an IPv6 address");
        }

        Ok(())
    }

    pub fn public_base_url(&self) -> String {
        let host = normalize_host(&self.public_host);
        if host.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("https://[{host}]:{}", self.port)
        } else {
            format!("https://{host}:{}", self.port)
        }
    }

    pub fn masked_token(&self) -> String {
        format!(
            "{}...",
            self.token_hash.get(..8).unwrap_or(&self.token_hash)
        )
    }
}

pub struct GeneratedToken {
    raw: String,
    hash: String,
}

impl GeneratedToken {
    pub fn new() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let raw = URL_SAFE_NO_PAD.encode(bytes);
        let hash = hex::encode(Sha256::digest(raw.as_bytes()));
        Self { raw, hash }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }
}

impl Default for GeneratedToken {
    fn default() -> Self {
        Self::new()
    }
}

pub fn verify_token(raw: &str, expected_hash: &str) -> bool {
    let Ok(expected): Result<[u8; 32], _> = hex::decode(expected_hash).and_then(|bytes| {
        bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)
    }) else {
        return false;
    };
    let actual: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
    actual.ct_eq(&expected).into()
}

fn normalize_host(host: &str) -> &str {
    host.trim().trim_start_matches('[').trim_end_matches(']')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_token_is_never_serialized_and_hash_verifies() {
        let generated = GeneratedToken::new();
        let config = SubscriptionConfig::new_disabled(generated.hash());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains(generated.raw()));
        assert!(verify_token(generated.raw(), &config.token_hash));
        assert!(!verify_token("wrong", &config.token_hash));
    }

    #[test]
    fn validation_rejects_acme_port_and_wrong_host_kind() {
        let mut config = SubscriptionConfig::new_disabled("00".repeat(32));
        config.enabled = true;
        config.port = 80;
        assert!(config.validate().is_err());
        config.port = 443;
        config.certificate_mode = CertificateMode::Ip;
        config.public_host = "example.com".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn atomic_round_trip_contains_only_operational_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subscription.json");
        let config = SubscriptionConfig::new_disabled("ab".repeat(32));
        config.save_atomic(&path).unwrap();
        assert_eq!(SubscriptionConfig::load_from(&path).unwrap(), Some(config));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn public_base_url_formats_domain_ipv4_and_ipv6_hosts() {
        let mut config = SubscriptionConfig::new_disabled("ab".repeat(32));
        config.public_host = "example.com".into();
        assert_eq!(config.public_base_url(), "https://example.com:443");

        config.public_host = "192.0.2.1".into();
        config.port = 8443;
        assert_eq!(config.public_base_url(), "https://192.0.2.1:8443");

        config.public_host = "[2001:db8::1]".into();
        assert_eq!(config.public_base_url(), "https://[2001:db8::1]:8443");
    }

    #[test]
    fn masked_token_handles_normal_and_short_hashes() {
        let mut config = SubscriptionConfig::new_disabled("ab".repeat(32));
        assert_eq!(config.masked_token(), "abababab...");

        config.token_hash = "abc".into();
        assert_eq!(config.masked_token(), "abc...");
    }
}
