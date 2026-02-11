use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use totp_rs::{Algorithm, Secret, TOTP};

pub struct TotpManager {
    totp: TOTP,
}

impl TotpManager {
    pub fn new(secret: &SecretString) -> Result<Self> {
        let secret_bytes = Secret::Encoded(secret.expose_secret().clone())
            .to_bytes()
            .map_err(|e| anyhow::anyhow!("❌ 无效的 TOTP 密钥: {}", e))?;

        let totp = TOTP::new(
            Algorithm::SHA512,
            6,
            1,
            30,
            secret_bytes,
            Some("wwps".to_string()),
            "admin".to_string(),
        )
        .map_err(|e| anyhow::anyhow!("❌ TOTP 初始化错误: {}", e))?;

        Ok(Self { totp })
    }

    pub fn verify(&self, token: &str) -> bool {
        self.totp.check_current(token).unwrap_or(false)
    }

    pub fn generate_new_secret() -> String {
        Secret::generate_secret().to_encoded().to_string()
    }
}
