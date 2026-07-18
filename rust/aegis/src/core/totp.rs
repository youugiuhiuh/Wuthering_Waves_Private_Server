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

    pub fn generate_current(&self) -> Result<String, std::time::SystemTimeError> {
        self.totp.generate_current()
    }

    pub fn verify_counter(&self, token: &str, unix_secs: u64) -> Option<u64> {
        let current = unix_secs / 30;
        [current.saturating_sub(1), current, current + 1]
            .into_iter()
            .find(|&counter| self.totp.generate(counter * 30) == token)
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

    #[test]
    fn verify_returns_true_for_valid_token() {
        let secret = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(secret.clone())).unwrap();

        let secret_bytes = totp_rs::Secret::Encoded(secret).to_bytes().unwrap();
        let totp = totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA512,
            6,
            1,
            30,
            secret_bytes,
            Some("wwps".to_string()),
            "admin".to_string(),
        )
        .unwrap();
        let token = totp.generate_current().unwrap();

        assert!(manager.verify(&token));
    }

    #[test]
    fn verify_returns_false_for_invalid_token() {
        let secret = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(secret)).unwrap();
        assert!(!manager.verify("000000"));
    }

    #[test]
    fn verify_counter_returns_matching_counter() {
        let encoded = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(encoded.clone())).unwrap();
        let now = 1_800_000_000;
        let token = manager.totp.generate(now);
        assert_eq!(manager.verify_counter(&token, now), Some(now / 30));
    }

    #[test]
    fn verify_counter_accepts_adjacent_skew() {
        let encoded = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(encoded.clone())).unwrap();
        let now = 1_800_000_000;
        let token_early = manager.totp.generate(now - 30);
        assert_eq!(
            manager.verify_counter(&token_early, now),
            Some(now / 30 - 1)
        );
        let token_late = manager.totp.generate(now + 30);
        assert_eq!(manager.verify_counter(&token_late, now), Some(now / 30 + 1));
    }

    #[test]
    fn verify_counter_rejects_invalid_token() {
        let manager = TotpManager::new(&secrecy::SecretString::from(
            TotpManager::generate_new_secret(),
        ))
        .unwrap();
        assert_eq!(manager.verify_counter("000000", 1_800_000_000), None);
    }

    #[test]
    fn verify_counter_rejects_token_outside_skew_window() {
        let encoded = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(encoded.clone())).unwrap();
        let now = 1_800_000_000;
        let token_early = manager.totp.generate(now - 60);
        assert_eq!(manager.verify_counter(&token_early, now), None);
        let token_late = manager.totp.generate(now + 60);
        assert_eq!(manager.verify_counter(&token_late, now), None);
    }

    #[test]
    fn raw_check_rejects_token_outside_skew_window() {
        let encoded = TotpManager::generate_new_secret();
        let bytes = Secret::Encoded(encoded).to_bytes().unwrap();

        let raw = TOTP::new(
            Algorithm::SHA512,
            6,
            1,
            30,
            bytes,
            Some("wwps".to_string()),
            "admin".to_string(),
        )
        .unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let token = raw.generate(now);

        assert!(raw.check(&token, now));
        assert!(!raw.check(&token, now + 90));
        assert!(!raw.check(&token, now.saturating_sub(90)));
    }
}
