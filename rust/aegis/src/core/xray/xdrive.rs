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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush_interval_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_window_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
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
            flush_interval_ms: None,
            eager_window_ms: None,
            concurrency: None,
        }
    }

    /// 预设构建器：标准 S3 兼容对象存储
    pub fn new_s3(
        endpoint: &str,
        region: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        path_style: bool,
        folder: &str,
    ) -> Self {
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: endpoint.to_string(),
                region: region.to_string(),
                bucket: bucket.to_string(),
                path_style,
            }),
            segment_bytes: Some(1024 * 1024),
            poll_interval_ms: Some(25),
            max_poll_interval_ms: Some(800),
            flush_interval_ms: None,
            eager_window_ms: None,
            concurrency: None,
        }
    }

    /// 预设构建器：Google Drive 网盘中转（遵循官方指南调优参数）
    pub fn new_google_drive(
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
        folder_id: &str,
    ) -> Self {
        Self {
            service: XdriveService::GoogleDrive,
            remote_folder: folder_id.to_string(),
            secrets: vec![
                client_id.to_string(),
                client_secret.to_string(),
                refresh_token.to_string(),
            ],
            s3: None,
            segment_bytes: Some(524288), // 512KB
            flush_interval_ms: Some(10), // 10ms
            poll_interval_ms: Some(50),  // 50ms
            max_poll_interval_ms: Some(200),
            eager_window_ms: Some(10000), // 10s
            concurrency: Some(64),
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
                if self.secrets.len() < 3 {
                    bail!(
                        "Google Drive 密钥参数 secrets 数量必须至少为 3（Client ID, Client Secret, Refresh Token）"
                    );
                }
                if self.secrets.iter().any(|s| s.trim().is_empty()) {
                    bail!("Google Drive Client ID, Client Secret 与 Refresh Token 均不能为空");
                }
                Ok(())
            }
            XdriveService::Template => Ok(()),
        }
    }

    /// 解析向导卡片文本输入（地区向导 Option A）
    pub fn parse_region_wizard_input(region: &str, input: &str) -> Result<Self> {
        let lines: Vec<&str> = input
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        match region.to_ascii_lowercase().as_str() {
            "ru" => {
                if lines.len() < 3 {
                    bail!(
                        "俄罗斯地区向导至少需要 3 行参数：Bucket名称、AccessKeyID、SecretAccessKey"
                    );
                }
                let bucket = lines[0];
                let ak = lines[1];
                let sk = lines[2];
                let folder = if lines.len() > 3 {
                    lines[3]
                } else {
                    "aegis-xdrive"
                };

                let config = Self {
                    service: XdriveService::S3,
                    remote_folder: folder.to_string(),
                    secrets: vec![ak.to_string(), sk.to_string()],
                    s3: Some(S3Config {
                        endpoint: "https://storage.yandexcloud.net".to_string(),
                        region: "ru-central1".to_string(),
                        bucket: bucket.to_string(),
                        path_style: true,
                    }),
                    segment_bytes: Some(1024 * 1024),
                    poll_interval_ms: Some(25),
                    max_poll_interval_ms: Some(800),
                    flush_interval_ms: None,
                    eager_window_ms: None,
                    concurrency: None,
                };
                config.validate()?;
                Ok(config)
            }
            "ir" => {
                if lines.len() < 3 {
                    bail!(
                        "伊朗地区向导至少需要 3 行参数：Bucket名称、AccessKeyID、SecretAccessKey"
                    );
                }
                let bucket = lines[0];
                let ak = lines[1];
                let sk = lines[2];
                let folder = if lines.len() > 3 {
                    lines[3]
                } else {
                    "aegis-xdrive"
                };

                let config = Self {
                    service: XdriveService::S3,
                    remote_folder: folder.to_string(),
                    secrets: vec![ak.to_string(), sk.to_string()],
                    s3: Some(S3Config {
                        endpoint: "https://s3.ir-thr-at1.arvanstorage.ir".to_string(),
                        region: "ir-thr-at1".to_string(),
                        bucket: bucket.to_string(),
                        path_style: true,
                    }),
                    segment_bytes: Some(1024 * 1024),
                    poll_interval_ms: Some(25),
                    max_poll_interval_ms: Some(800),
                    flush_interval_ms: None,
                    eager_window_ms: None,
                    concurrency: None,
                };
                config.validate()?;
                Ok(config)
            }
            "cn" => {
                if lines.len() < 4 {
                    bail!(
                        "中国大陆地区向导至少需要 4 行参数：Cloudflare AccountID、Bucket名称、AccessKeyID、SecretAccessKey"
                    );
                }
                let account_id = lines[0];
                let bucket = lines[1];
                let ak = lines[2];
                let sk = lines[3];
                let folder = if lines.len() > 4 {
                    lines[4]
                } else {
                    "aegis-xdrive"
                };

                let config = Self::new_cloudflare_r2(account_id, bucket, ak, sk, folder);
                config.validate()?;
                Ok(config)
            }
            "custom" => {
                if lines.len() < 5 {
                    bail!(
                        "自定义 S3 向导至少需要 5 行参数：Endpoint、Region、Bucket名称、AccessKeyID、SecretAccessKey"
                    );
                }
                let endpoint = lines[0];
                let s3_region = lines[1];
                let bucket = lines[2];
                let ak = lines[3];
                let sk = lines[4];
                let folder = if lines.len() > 5 {
                    lines[5]
                } else {
                    "aegis-xdrive"
                };

                let config = Self {
                    service: XdriveService::S3,
                    remote_folder: folder.to_string(),
                    secrets: vec![ak.to_string(), sk.to_string()],
                    s3: Some(S3Config {
                        endpoint: endpoint.to_string(),
                        region: s3_region.to_string(),
                        bucket: bucket.to_string(),
                        path_style: true,
                    }),
                    segment_bytes: Some(1024 * 1024),
                    poll_interval_ms: Some(25),
                    max_poll_interval_ms: Some(800),
                    flush_interval_ms: None,
                    eager_window_ms: None,
                    concurrency: None,
                };
                config.validate()?;
                Ok(config)
            }
            other => bail!("未知地区类型: {other}"),
        }
    }

    /// 提取 Cloudflare 账户 ID（支持纯 32 位十六进制 ID 或完整终结点 URL）
    pub fn extract_cloudflare_account_id(input: &str) -> Option<String> {
        let trimmed = input.trim();
        // 尝试从 https://<id>.r2.cloudflarestorage.com 或类似 URL 中提取
        if let Some(pos) = trimmed.find(".r2.cloudflarestorage.com") {
            let prefix = &trimmed[..pos];
            let id = prefix.rsplit('/').next().unwrap_or(prefix);
            let id = id.rsplit(':').next().unwrap_or(id);
            if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(id.to_ascii_lowercase());
            }
        }
        // 纯 32 位十六进制 ID
        if trimmed.len() == 32 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(trimmed.to_ascii_lowercase());
        }
        None
    }

    /// 智能解析用户直接复制的 Cloudflare R2 令牌创建完成页全文
    /// 返回 (account_id, access_key_id, secret_access_key)
    pub fn extract_cloudflare_r2_page_tokens(text: &str) -> Option<(String, String, String)> {
        // 优先从终结点 URL 中提取 account_id
        let mut account_id = None;
        for line in text.lines() {
            if line.contains(".r2.cloudflarestorage.com")
                && let Some(id) = Self::extract_cloudflare_account_id(line)
            {
                account_id = Some(id);
                break;
            }
        }

        // 提取 access_key_id 和 secret_access_key
        let mut ak = None;
        let mut sk = None;
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        for (i, line) in lines.iter().enumerate() {
            if (line.contains("访问密钥 ID") || line.contains("Access Key ID"))
                && let Some(next) = lines.get(i + 1)
                && next.len() == 32
                && next.chars().all(|c| c.is_ascii_hexdigit())
            {
                ak = Some(next.to_string());
            }
            if (line.contains("机密访问密钥") || line.contains("Secret Access Key"))
                && let Some(next) = lines.get(i + 1)
                && next.len() == 64
                && next.chars().all(|c| c.is_ascii_hexdigit())
            {
                sk = Some(next.to_string());
            }
        }

        match (account_id, ak, sk) {
            (Some(acc), Some(a), Some(s)) => Some((acc, a, s)),
            _ => None,
        }
    }

    /// 智能解析 Google Drive 凭据全文（支持 key-value、JSON、分步文本与直接粘贴）
    /// 返回 (client_id, client_secret, refresh_token, folder_id)
    pub fn extract_google_drive_tokens(
        text: &str,
    ) -> Option<(String, String, String, Option<String>)> {
        let mut client_id = None;
        let mut client_secret = None;
        let mut refresh_token = None;
        let mut folder_id = None;

        let clean_val = |raw: &str| -> String {
            raw.trim()
                .trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ';')
                .trim()
                .to_string()
        };

        let extract_folder_from_url = |val: &str| -> String {
            if let Some(pos) = val.find("/folders/") {
                let after = &val[pos + 9..];
                let end = after
                    .find(|c: char| {
                        c == '?' || c == '/' || c == '"' || c == '\'' || c.is_whitespace()
                    })
                    .unwrap_or(after.len());
                after[..end].trim().to_string()
            } else {
                clean_val(val)
            }
        };

        // 1. 尝试解析 JSON 格式
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            let search_obj = if let Some(installed) = v.get("installed") {
                installed
            } else if let Some(web) = v.get("web") {
                web
            } else {
                &v
            };

            if let Some(cid) = search_obj.get("client_id").and_then(|s| s.as_str()) {
                client_id = Some(cid.to_string());
            }
            if let Some(cs) = search_obj.get("client_secret").and_then(|s| s.as_str()) {
                client_secret = Some(cs.to_string());
            }
            if let Some(rt) = search_obj
                .get("refresh_token")
                .or_else(|| v.get("refresh_token"))
                .and_then(|s| s.as_str())
            {
                refresh_token = Some(rt.to_string());
            }
            if let Some(fid) = search_obj
                .get("folder_id")
                .or_else(|| v.get("folder_id"))
                .and_then(|s| s.as_str())
            {
                folder_id = Some(extract_folder_from_url(fid));
            }
        }

        // 2. 逐行分析（Key-Value、标签下一行值以及特征值嗅探）
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        for (i, line) in lines.iter().enumerate() {
            let lower = line.to_ascii_lowercase();

            let (key_part, val_part) = if let Some(pos) = line.find(['=', ':']) {
                let k = line[..pos].trim().to_ascii_lowercase();
                let v = line[pos + 1..].trim();
                (Some(k), Some(v))
            } else {
                (None, None)
            };

            // Client ID 提取
            if client_id.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part)
                    && (k.contains("client_id")
                        || k.contains("client id")
                        || k.contains("客户端 id")
                        || k.contains("客户端id"))
                    && !v.is_empty()
                {
                    client_id = Some(clean_val(v));
                }
                if client_id.is_none()
                    && (lower == "client id"
                        || lower == "client_id"
                        || lower == "客户端 id"
                        || lower == "客户端id")
                    && let Some(next) = lines.get(i + 1)
                {
                    client_id = Some(clean_val(next));
                }
            }

            // Client Secret 提取
            if client_secret.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part)
                    && (k.contains("client_secret")
                        || k.contains("client secret")
                        || k.contains("客户端密钥")
                        || k.contains("客户端机密")
                        || k == "secret")
                    && !v.is_empty()
                {
                    client_secret = Some(clean_val(v));
                }
                if client_secret.is_none()
                    && (lower == "client secret"
                        || lower == "client_secret"
                        || lower == "客户端密钥"
                        || lower == "secret")
                    && let Some(next) = lines.get(i + 1)
                {
                    client_secret = Some(clean_val(next));
                }
            }

            // Refresh Token 提取
            if refresh_token.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part)
                    && (k.contains("refresh_token")
                        || k.contains("refresh token")
                        || k.contains("刷新令牌"))
                    && !v.is_empty()
                {
                    refresh_token = Some(clean_val(v));
                }
                if refresh_token.is_none()
                    && (lower == "refresh token" || lower == "refresh_token" || lower == "刷新令牌")
                    && let Some(next) = lines.get(i + 1)
                {
                    refresh_token = Some(clean_val(next));
                }
            }

            // Folder ID 提取
            if folder_id.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part)
                    && (k.contains("folder_id")
                        || k.contains("folder id")
                        || k.contains("文件夹 id")
                        || k.contains("文件夹id")
                        || k == "folder")
                    && !v.is_empty()
                {
                    folder_id = Some(extract_folder_from_url(v));
                }
                if folder_id.is_none()
                    && (lower == "folder id"
                        || lower == "folder_id"
                        || lower == "文件夹 id"
                        || lower == "folder")
                    && let Some(next) = lines.get(i + 1)
                {
                    folder_id = Some(extract_folder_from_url(next));
                }
            }

            // 凭据特征嗅探（按单词）
            for word in line.split_whitespace() {
                let cleaned_word = clean_val(word);
                if client_id.is_none() && cleaned_word.contains(".apps.googleusercontent.com") {
                    client_id = Some(cleaned_word.clone());
                }
                if client_secret.is_none() && cleaned_word.starts_with("GOCSPX-") {
                    client_secret = Some(cleaned_word.clone());
                }
                if refresh_token.is_none()
                    && (cleaned_word.starts_with("1//") || cleaned_word.starts_with("1/"))
                    && cleaned_word.len() > 15
                {
                    refresh_token = Some(cleaned_word.clone());
                }
                if folder_id.is_none() && cleaned_word.contains("/folders/") {
                    folder_id = Some(extract_folder_from_url(&cleaned_word));
                }
            }
        }

        match (client_id, client_secret, refresh_token) {
            (Some(cid), Some(cs), Some(rt)) => {
                if !cid.is_empty() && !cs.is_empty() && !rt.is_empty() {
                    Some((cid, cs, rt, folder_id.filter(|f| !f.is_empty())))
                } else {
                    None
                }
            }
            _ => None,
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
            flush_interval_ms: None,
            eager_window_ms: None,
            concurrency: None,
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

    #[test]
    fn test_parse_ru_wizard_input() {
        let input = "my-yandex-bucket\nYCAJEm4xxx\nYCPm89yyy\n";
        let config = XdriveConfig::parse_region_wizard_input("ru", input).unwrap();
        assert_eq!(config.service, XdriveService::S3);
        assert_eq!(config.remote_folder, "aegis-xdrive");
        assert_eq!(config.secrets, vec!["YCAJEm4xxx", "YCPm89yyy"]);
        let s3 = config.s3.unwrap();
        assert_eq!(s3.endpoint, "https://storage.yandexcloud.net");
        assert_eq!(s3.region, "ru-central1");
        assert_eq!(s3.bucket, "my-yandex-bucket");
        assert!(s3.path_style);
    }

    #[test]
    fn test_parse_ir_wizard_input() {
        let input = "arvan-bucket\nARVANAK\nARVANSK\ncustom-folder";
        let config = XdriveConfig::parse_region_wizard_input("ir", input).unwrap();
        assert_eq!(config.remote_folder, "custom-folder");
        let s3 = config.s3.unwrap();
        assert_eq!(s3.endpoint, "https://s3.ir-thr-at1.arvanstorage.ir");
        assert_eq!(s3.region, "ir-thr-at1");
    }

    #[test]
    fn test_parse_cn_wizard_input() {
        let input = "a1b2c3d4e5f6\nr2-bucket\nR2AK\nR2SK\n";
        let config = XdriveConfig::parse_region_wizard_input("cn", input).unwrap();
        let s3 = config.s3.unwrap();
        assert_eq!(s3.endpoint, "https://a1b2c3d4e5f6.r2.cloudflarestorage.com");
        assert_eq!(s3.region, "auto");
        assert_eq!(s3.bucket, "r2-bucket");
    }

    #[test]
    fn test_parse_custom_wizard_input() {
        let input = "https://my-s3.example.com\nus-west-1\nmy-b\nMYAK\nMYSK\n";
        let config = XdriveConfig::parse_region_wizard_input("custom", input).unwrap();
        let s3 = config.s3.unwrap();
        assert_eq!(s3.endpoint, "https://my-s3.example.com");
        assert_eq!(s3.region, "us-west-1");
        assert_eq!(s3.bucket, "my-b");
    }

    #[test]
    fn test_parse_wizard_input_rejects_insufficient_lines() {
        assert!(XdriveConfig::parse_region_wizard_input("ru", "only-bucket").is_err());
        assert!(XdriveConfig::parse_region_wizard_input("cn", "acc\nbucket\nak").is_err());
    }

    #[test]
    fn test_extract_cloudflare_account_id() {
        let url = "https://a5be0a9d14aa44e51692d81782d83fe6.r2.cloudflarestorage.com";
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id(url),
            Some("a5be0a9d14aa44e51692d81782d83fe6".to_string())
        );

        let raw = "a5be0a9d14aa44e51692d81782d83fe6";
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id(raw),
            Some("a5be0a9d14aa44e51692d81782d83fe6".to_string())
        );

        assert_eq!(XdriveConfig::extract_cloudflare_account_id("invalid"), None);
    }

    #[test]
    fn test_extract_cloudflare_r2_page_tokens() {
        let sample = r#"
R2 Account Token 已成功创建
摘要：
权限：
允许创建、列出和删除存储桶，编辑存储桶配置，读取、写入和列出对象，以及对数据目录表和相关元数据进行读取和写入访问。

存储桶：
此帐户上的所有 R2 存储桶
TTL
可用时间 Nov 6, 2026 - Nov 6, 2027
使用此令牌对 Cloudflare API 进行身份验证：
令牌值
cfat_mocktoken1234567890abcdef1234567890

为 S3 客户端使用以下凭据：
访问密钥 ID
b65ef1b0f6dcc85d9c2e42a92610fe14

机密访问密钥
4b07c8ef6340317e0b51fae296837bfdbddbfcf92e071727c955da3597dff758

为 S3 客户端使用管辖权地特定的终结点：
默认欧盟 (EU)美国 (US)
https://a5be0a9d14aa44e51692d81782d83fe6.r2.cloudflarestorage.com
"#;
        let (acc, ak, sk) = XdriveConfig::extract_cloudflare_r2_page_tokens(sample).unwrap();
        assert_eq!(acc, "a5be0a9d14aa44e51692d81782d83fe6");
        assert_eq!(ak, "b65ef1b0f6dcc85d9c2e42a92610fe14");
        assert_eq!(
            sk,
            "4b07c8ef6340317e0b51fae296837bfdbddbfcf92e071727c955da3597dff758"
        );
    }

    #[test]
    fn test_valid_google_drive_preset() {
        let config = XdriveConfig::new_google_drive(
            "test-client-id.apps.googleusercontent.com",
            "GOCSPX-testclientsecret",
            "1//04testrefreshtoken",
            "1AbCdEfGhIjKlMnOpQrStUvWxYz",
        );

        assert_eq!(config.service, XdriveService::GoogleDrive);
        assert_eq!(config.remote_folder, "1AbCdEfGhIjKlMnOpQrStUvWxYz");
        assert_eq!(
            config.secrets,
            vec![
                "test-client-id.apps.googleusercontent.com",
                "GOCSPX-testclientsecret",
                "1//04testrefreshtoken"
            ]
        );
        assert!(config.s3.is_none());
        assert_eq!(config.segment_bytes, Some(524288));
        assert_eq!(config.flush_interval_ms, Some(10));
        assert_eq!(config.poll_interval_ms, Some(50));
        assert_eq!(config.max_poll_interval_ms, Some(200));
        assert_eq!(config.eager_window_ms, Some(10000));
        assert_eq!(config.concurrency, Some(64));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_google_drive_validation_rejects_missing_secrets() {
        // 少于 3 个凭据参数
        let mut config = XdriveConfig::new_google_drive("cid", "secret", "refresh", "folder");
        config.secrets = vec!["cid".to_string(), "secret".to_string()];
        assert!(config.validate().is_err());

        // 包含空字符串凭据
        let mut config2 = XdriveConfig::new_google_drive("cid", "   ", "refresh", "folder");
        config2.secrets[1] = "".to_string();
        assert!(config2.validate().is_err());

        // remote_folder 为空
        let config3 = XdriveConfig::new_google_drive("cid", "secret", "refresh", "   ");
        assert!(config3.validate().is_err());
    }

    #[test]
    fn test_extract_google_drive_tokens() {
        // 1. Key-Value 格式
        let text_kv = r#"
CLIENT_ID = 123456789.apps.googleusercontent.com
CLIENT_SECRET = GOCSPX-secret123
REFRESH_TOKEN = 1//04abcdefghij
FOLDER_ID = folder_abc_123
"#;
        let (cid, secret, refresh, folder) =
            XdriveConfig::extract_google_drive_tokens(text_kv).expect("KV 凭据解析失败");
        assert_eq!(cid, "123456789.apps.googleusercontent.com");
        assert_eq!(secret, "GOCSPX-secret123");
        assert_eq!(refresh, "1//04abcdefghij");
        assert_eq!(folder, Some("folder_abc_123".to_string()));

        // 2. JSON 格式与 URL 解析
        let text_json = r#"{
            "client_id": "json-cid.apps.googleusercontent.com",
            "client_secret": "GOCSPX-jsonsecret",
            "refresh_token": "1//04jsonrefresh",
            "folder_id": "https://drive.google.com/drive/folders/1XYZFolder987?usp=sharing"
        }"#;
        let (cid, secret, refresh, folder) =
            XdriveConfig::extract_google_drive_tokens(text_json).expect("JSON 凭据解析失败");
        assert_eq!(cid, "json-cid.apps.googleusercontent.com");
        assert_eq!(secret, "GOCSPX-jsonsecret");
        assert_eq!(refresh, "1//04jsonrefresh");
        assert_eq!(folder, Some("1XYZFolder987".to_string()));

        // 3. 无 folder_id 的有效提取
        let text_no_folder = "Client ID: cid\nClient Secret: sec\nRefresh Token: ref\n";
        let (cid, secret, refresh, folder) =
            XdriveConfig::extract_google_drive_tokens(text_no_folder).expect("无 folder 解析失败");
        assert_eq!(cid, "cid");
        assert_eq!(secret, "sec");
        assert_eq!(refresh, "ref");
        assert_eq!(folder, None);

        // 4. 缺少必要字段应返回 None
        let text_missing = "CLIENT_ID: cid\nFOLDER_ID: fld\n";
        assert!(XdriveConfig::extract_google_drive_tokens(text_missing).is_none());
    }
}
