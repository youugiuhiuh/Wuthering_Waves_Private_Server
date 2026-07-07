use minisign_verify::{PublicKey, Signature};

static PUBLIC_KEY: &str = "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf";

pub struct MinisigInfo {
    pub trusted_comment: String,
}

pub fn verify_minisign(binary_data: &[u8], signature_data: &[u8]) -> Result<MinisigInfo, String> {
    let pk = PublicKey::from_base64(PUBLIC_KEY).map_err(|e| format!("公钥解码失败: {}", e))?;
    let sig_str = std::str::from_utf8(signature_data)
        .map_err(|e| format!("签名数据不是有效 UTF-8: {}", e))?;
    let sig = Signature::decode(sig_str).map_err(|e| format!("签名解码失败: {}", e))?;
    match pk.verify(binary_data, &sig, true) {
        Ok(()) => {
            let trusted_comment = sig.trusted_comment().to_string();
            Ok(MinisigInfo { trusted_comment })
        }
        Err(e) => Err(format!("minisign 验证失败: {}", e)),
    }
}

pub fn parse_trusted_comment(comment: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = comment.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("无效的可信注释格式: {}", comment));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trusted_comment_valid() {
        let (ver, asset) = parse_trusted_comment("3.2.10:sub-server").unwrap();
        assert_eq!(ver, "3.2.10");
        assert_eq!(asset, "sub-server");
    }

    #[test]
    fn test_parse_trusted_comment_multi_colon() {
        let (ver, asset) = parse_trusted_comment("3.2.10:sub-server:extra").unwrap();
        assert_eq!(ver, "3.2.10");
        assert_eq!(asset, "sub-server:extra");
    }

    #[test]
    fn test_parse_trusted_comment_no_colon() {
        assert!(parse_trusted_comment("invalid").is_err());
    }

    #[test]
    fn test_parse_trusted_comment_empty() {
        assert!(parse_trusted_comment("").is_err());
    }
}
