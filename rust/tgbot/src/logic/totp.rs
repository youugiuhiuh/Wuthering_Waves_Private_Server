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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_new_secret_returns_valid_base32() {
        let secret = TotpManager::generate_new_secret();
        assert!(!secret.is_empty());
        assert!(secret.len() >= 16);
        assert!(
            secret
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
        let _manager = TotpManager::new(&secrecy::SecretString::from(secret)).unwrap();
    }

    #[test]
    fn new_accepts_valid_base32_secret() {
        let secret = TotpManager::generate_new_secret();
        let result = TotpManager::new(&secrecy::SecretString::from(secret));
        assert!(result.is_ok());
    }

    #[test]
    fn new_rejects_secret_with_trailing_newline() {
        let secret = TotpManager::generate_new_secret();
        let secret_with_newline = format!("{}\n", secret);
        let result = TotpManager::new(&secrecy::SecretString::from(secret_with_newline));
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("base32"));
    }

    #[test]
    fn new_rejects_invalid_base32() {
        let result = TotpManager::new(&secrecy::SecretString::from(
            "not-valid-base32!!!".to_string(),
        ));
        assert!(result.is_err());
    }

    #[test]
    fn new_accepts_trimmed_secret_that_originally_had_whitespace() {
        let secret = TotpManager::generate_new_secret();
        let with_whitespace = format!("  {}  \n\t ", secret);
        let trimmed = with_whitespace.trim();
        let result = TotpManager::new(&secrecy::SecretString::from(trimmed.to_string()));
        assert!(result.is_ok());
    }
}
