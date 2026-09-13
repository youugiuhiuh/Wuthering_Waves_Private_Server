use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use totp_rs::{Algorithm, Builder, Secret, Totp};

pub struct TotpManager {
    totp: Totp,
}

impl TotpManager {
    pub fn new(secret: &SecretString) -> Result<Self> {
        let secret = Secret::try_from_base32(secret.expose_secret())
            .map_err(|e| anyhow::anyhow!("❌ 无效的 TOTP 密钥: {}", e))?;

        let totp = Builder::new()
            .with_algorithm(Algorithm::SHA512)
            .with_digits(6)
            .with_skew(1)
            .with_step_duration(30)
            .with_secret(secret)
            .with_issuer(Some("wwps"))
            .with_account_name("admin")
            .build()
            .map_err(|e| anyhow::anyhow!("❌ TOTP 初始化错误: {}", e))?;

        Ok(Self { totp })
    }

    pub fn verify(&self, token: &str) -> bool {
        self.totp.check_current(token).is_some()
    }

    pub fn generate_current(&self) -> String {
        self.totp.generate_current().to_string()
    }

    pub fn generate_new_secret() -> String {
        Secret::generate().to_base32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立的参考实现：不经 TotpManager，直接按相同参数构建，用于交叉校验。
    fn reference_totp(secret: &str) -> Totp {
        Builder::new()
            .with_algorithm(Algorithm::SHA512)
            .with_digits(6)
            .with_skew(1)
            .with_step_duration(30)
            .with_secret(Secret::try_from_base32(secret).unwrap())
            .with_issuer(Some("wwps"))
            .with_account_name("admin")
            .build()
            .unwrap()
    }

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

        let token = reference_totp(&secret).generate_current().to_string();

        assert!(manager.verify(&token));
    }

    #[test]
    fn verify_returns_false_for_invalid_token() {
        let secret = TotpManager::generate_new_secret();
        let manager = TotpManager::new(&secrecy::SecretString::from(secret)).unwrap();
        assert!(!manager.verify("000000"));
    }

    #[test]
    fn raw_check_rejects_token_outside_skew_window() {
        let raw = reference_totp(&TotpManager::generate_new_secret());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let token = raw.generate(now).to_string();

        assert!(raw.check(&token, now).is_some());
        assert!(raw.check(&token, now + 90).is_none());
        assert!(raw.check(&token, now.saturating_sub(90)).is_none());
    }

    /// 冻结向量：totp-rs 5.7.2 在 t=1_700_000_000 对固定密钥产出 `720184`。
    /// 若升级后此处失败，说明令牌派生发生变化，现有用户密钥将全部失效。
    #[test]
    fn token_derivation_matches_frozen_v5_vector() {
        let encoded = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
        let raw = reference_totp(encoded);

        assert_eq!(raw.generate(1_700_000_000).to_string(), "720184");
        assert_eq!(
            raw.to_url().unwrap(),
            "otpauth://totp/wwps:admin\
             ?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP\
             &algorithm=SHA512&issuer=wwps"
        );

        // 同一密钥经 TotpManager 装载后，与参考实现产出相同的当前码
        // （注意：verify() 按墙钟时间校验，不能用冻结的 1_700_000_000 向量）
        let manager = TotpManager::new(&secrecy::SecretString::from(encoded.to_string())).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let current = raw.generate(now).to_string();
        assert_eq!(manager.generate_current(), current);
        assert!(manager.verify(&current));
    }
}
