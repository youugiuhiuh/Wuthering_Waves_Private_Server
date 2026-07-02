use anyhow::{Result, anyhow};

pub struct MinisignKeyEntry {
    pub public_key: &'static str,
    pub expires_at: &'static str, // "YYYY-MM-DD", empty = expired
}

pub const MINISIGN_PUBLIC_KEYS: &[MinisignKeyEntry] = &[];

pub struct MinisigInfo {
    pub trusted_comment: String,
}

fn key_expired(expires_at: &str) -> bool {
    if expires_at.is_empty() {
        return true;
    }
    // ISO 8601 dates sort lexicographically
    let now_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
    now_str.as_str() > expires_at
}

pub fn verify_minisign(
    data: &[u8],
    sig_str: &str,
    pub_keys: &[MinisignKeyEntry],
) -> Result<MinisigInfo> {
    let sig =
        minisign_verify::Signature::decode(sig_str).map_err(|e| anyhow!("解析签名失败: {}", e))?;

    for entry in pub_keys {
        if key_expired(entry.expires_at) {
            continue;
        }
        let pub_key = match minisign_verify::PublicKey::from_base64(entry.public_key) {
            Ok(k) => k,
            Err(_) => continue,
        };
        if pub_key.verify(data, &sig, false).is_ok() {
            return Ok(MinisigInfo {
                trusted_comment: sig.trusted_comment().to_string(),
            });
        }
    }

    Err(anyhow!("Minisign 验证失败: 无匹配公钥"))
}

pub fn parse_trusted_comment(comment: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = comment.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(anyhow!("无效的可信注释格式: {}", comment));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trusted_comment_valid() {
        let (ver, name) = parse_trusted_comment("v3.1.8:aegis").unwrap();
        assert_eq!(ver, "v3.1.8");
        assert_eq!(name, "aegis");
    }

    #[test]
    fn test_parse_trusted_comment_no_colon() {
        assert!(parse_trusted_comment("no-colon").is_err());
    }

    #[test]
    fn test_parse_trusted_comment_empty() {
        assert!(parse_trusted_comment("").is_err());
    }

    #[test]
    fn test_parse_trusted_comment_multi_colon() {
        let (ver, name) = parse_trusted_comment("v1.0.0:file:extra").unwrap();
        assert_eq!(ver, "v1.0.0");
        assert_eq!(name, "file:extra");
    }
}
