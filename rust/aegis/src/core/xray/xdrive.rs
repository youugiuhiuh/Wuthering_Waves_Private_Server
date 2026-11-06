use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdriveService {
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "google-drive")]
    GoogleDrive,
    #[serde(rename = "template")]
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub path_style: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XdriveConfig {
    pub service: XdriveService,
    pub remote_folder: String,
    pub secrets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3: Option<S3Config>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_bytes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_poll_interval_ms: Option<u32>,
}

impl XdriveConfig {
    /// 预设构建器：Cloudflare R2（首选高抗封锁 S3 兼容后端）
    pub fn new_cloudflare_r2(
        account_id: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        folder: &str,
    ) -> Self {
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: format!("https://{account_id}.r2.cloudflarestorage.com"),
                region: "auto".to_string(),
                bucket: bucket.to_string(),
                path_style: true,
            }),
            segment_bytes: Some(1024 * 1024), // 默认 1MB
            poll_interval_ms: Some(25),       // 低延迟轮询
            max_poll_interval_ms: Some(800),
        }
    }

    /// 校验配置合法性（对齐 Xray-core transport/internet/xdrive/s3.go 规范）
    pub fn validate(&self) -> Result<()> {
        if self.remote_folder.trim().is_empty() {
            bail!("remoteFolder 不能为空");
        }
        if self.remote_folder.contains("..") {
            bail!("remoteFolder 不能包含相对路径 '..'");
        }

        match self.service {
            XdriveService::S3 => {
                let s3 = self
                    .s3
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("S3 服务模式必须提供 s3 配置对象"))?;

                if !s3.endpoint.starts_with("http://") && !s3.endpoint.starts_with("https://") {
                    bail!("S3 endpoint 必须以 http:// 或 https:// 开头");
                }
                if s3.bucket.trim().is_empty() || s3.bucket.contains('/') {
                    bail!("S3 bucket 名称不能为空且不得包含路径分隔符 '/'");
                }
                if self.secrets.len() < 2 || self.secrets.len() > 3 {
                    bail!("S3 密钥参数 secrets 数量必须为 2（AK, SK）或 3（含 SessionToken）");
                }
                if self.secrets[0].trim().is_empty() || self.secrets[1].trim().is_empty() {
                    bail!("S3 AccessKeyID 与 SecretAccessKey 不能为空");
                }
                Ok(())
            }
            XdriveService::Local => Ok(()),
            XdriveService::GoogleDrive => {
                // 预留枚举，当前未接通
                Ok(())
            }
            XdriveService::Template => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_cloudflare_r2_preset() {
        let config = XdriveConfig::new_cloudflare_r2(
            "acc123",
            "my-bucket",
            "AKIDEXAMPLE",
            "SECRETKEYEXAMPLE",
            "aegis-xdrive",
        );
        assert_eq!(config.service, XdriveService::S3);
        assert_eq!(config.remote_folder, "aegis-xdrive");
        assert_eq!(config.secrets, vec!["AKIDEXAMPLE", "SECRETKEYEXAMPLE"]);
        let s3 = config.s3.as_ref().expect("s3 config must be present");
        assert_eq!(s3.endpoint, "https://acc123.r2.cloudflarestorage.com");
        assert_eq!(s3.bucket, "my-bucket");
        assert_eq!(s3.region, "auto");
        assert!(s3.path_style);
        assert_eq!(config.poll_interval_ms, Some(25));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_s3_validation_rejects_missing_s3_object() {
        let config = XdriveConfig {
            service: XdriveService::S3,
            remote_folder: "data".to_string(),
            secrets: vec!["ak".to_string(), "sk".to_string()],
            s3: None,
            segment_bytes: None,
            poll_interval_ms: None,
            max_poll_interval_ms: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_s3_validation_rejects_invalid_endpoint() {
        let mut config = XdriveConfig::new_cloudflare_r2("acc", "b", "ak", "sk", "folder");
        if let Some(ref mut s3) = config.s3 {
            s3.endpoint = "ftp://invalid.endpoint".to_string();
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_s3_validation_rejects_slash_in_bucket() {
        let mut config = XdriveConfig::new_cloudflare_r2("acc", "b/invalid", "ak", "sk", "folder");
        if let Some(ref mut s3) = config.s3 {
            s3.bucket = "b/invalid".to_string();
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_s3_validation_rejects_invalid_secret_count() {
        let mut config = XdriveConfig::new_cloudflare_r2("acc", "b", "ak", "sk", "folder");
        config.secrets = vec!["only-one".to_string()];
        assert!(config.validate().is_err());
    }
}
