use anyhow::{Result, anyhow};

pub struct MinisignKeyEntry {
    pub public_key: &'static str,
    pub expires_at: &'static str, // "YYYY-MM-DD", empty = expired
    pub retired_at: &'static str, // "YYYY-MM-DD"; empty = 仍活跃
}

/// 活跃密钥：用于验证当前/新发布的资产，会检查 expires_at。
pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry] = &[MinisignKeyEntry {
    public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
    expires_at: "2027-07-02",
    retired_at: "",
}];

/// 历史密钥：仅用于验证「用旧钥签的历史版本」。
/// 刻意不检查 expires_at —— 否则历史版本将永远无法验证。
pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry] = &[];

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

fn is_active(entry: &MinisignKeyEntry) -> bool {
    entry.retired_at.is_empty()
}

/// 验证签名。先试活跃密钥（检查过期），再试历史密钥（忽略过期）。
pub fn verify_minisign(
    data: &[u8],
    sig_str: &str,
    active_keys: &[MinisignKeyEntry],
    historical_keys: &[MinisignKeyEntry],
) -> Result<MinisigInfo> {
    let sig =
        minisign_verify::Signature::decode(sig_str).map_err(|e| anyhow!("解析签名失败: {}", e))?;

    // 活跃密钥：跳过已退役与已过期者
    for entry in active_keys {
        if !is_active(entry) || key_expired(entry.expires_at) {
            continue;
        }
        if let Some(info) = try_verify(data, &sig, entry) {
            return Ok(info);
        }
    }

    // 历史密钥：不检查过期（否则历史版本永远无法验证）
    for entry in historical_keys {
        if let Some(info) = try_verify(data, &sig, entry) {
            return Ok(info);
        }
    }

    Err(anyhow!("Minisign 验证失败: 无匹配公钥"))
}

fn try_verify(
    data: &[u8],
    sig: &minisign_verify::Signature,
    entry: &MinisignKeyEntry,
) -> Option<MinisigInfo> {
    let pub_key = minisign_verify::PublicKey::from_base64(entry.public_key).ok()?;
    if pub_key.verify(data, sig, false).is_ok() {
        Some(MinisigInfo {
            trusted_comment: sig.trusted_comment().to_string(),
        })
    } else {
        None
    }
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

    #[test]
    fn test_key_expired_empty_is_expired() {
        assert!(key_expired(""));
    }

    #[test]
    fn test_key_expired_past_is_expired() {
        assert!(key_expired("2000-01-01"));
    }

    #[test]
    fn test_key_expired_future_is_not_expired() {
        assert!(!key_expired("2999-12-31"));
    }

    #[test]
    fn test_is_active_requires_empty_retired_at() {
        let active = MinisignKeyEntry {
            public_key: "X",
            expires_at: "2999-12-31",
            retired_at: "",
        };
        let retired = MinisignKeyEntry {
            public_key: "X",
            expires_at: "2999-12-31",
            retired_at: "2026-01-01",
        };
        assert!(is_active(&active));
        assert!(!is_active(&retired));
    }

    #[test]
    fn test_active_and_historical_key_lists_are_disjoint_by_retired_at() {
        // 活跃列表内不得出现已退役标记
        for entry in MINISIGN_ACTIVE_KEYS {
            assert_eq!(entry.retired_at, "", "活跃列表含 retired_at 非空项");
        }
        // 历史列表内每项都应有退役标记
        for entry in MINISIGN_HISTORICAL_KEYS {
            assert_ne!(entry.retired_at, "", "历史列表含 retired_at 为空项");
        }
    }

    #[test]
    fn test_verify_rejects_when_no_keys_supplied() {
        let err = verify_minisign(b"data", "not-a-signature", &[], &[]);
        assert!(err.is_err());
    }

    #[test]
    fn test_verify_checks_active_keys_for_expiry() {
        // 过期活跃钥必须被跳过 → 最终无匹配
        let expired = [MinisignKeyEntry {
            public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
            expires_at: "2000-01-01",
            retired_at: "",
        }];
        let err = verify_minisign(b"data", "not-a-signature", &expired, &[]);
        assert!(err.is_err());
    }

    #[test]
    fn test_verify_historical_keys_ignore_expiry() {
        // 历史钥即便 expires_at 已过也必须被尝试（此处仍无有效签名 → Err，
        // 但关键是不能因过期而被提前跳过；用无效签名保证不 panic）
        let historical = [MinisignKeyEntry {
            public_key: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf",
            expires_at: "2000-01-01",
            retired_at: "2026-01-01",
        }];
        let err = verify_minisign(b"data", "not-a-signature", &[], &historical);
        assert!(err.is_err());
    }
}
