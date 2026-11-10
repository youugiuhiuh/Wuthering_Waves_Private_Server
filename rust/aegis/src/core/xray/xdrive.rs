use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub use crate::shared::handlers::message::extract_onedrive_tokens;

/// rclone 默认公开 Client ID（适用于个人免费版 OneDrive）
pub const ONEDRIVE_DEFAULT_CLIENT_ID: &str = "b15665d9-eda6-4092-8539-0eec376afd59";
/// rclone 默认公开 Client Secret（解密值）
pub const ONEDRIVE_DEFAULT_CLIENT_SECRET: &str = "qtyfaBBYA403=unZUP40~_#";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdriveService {
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "google-drive")]
    GoogleDrive,
    #[serde(rename = "onedrive")]
    OneDrive,
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
        Self::new_cloudflare_r2_with_endpoint(
            &format!("https://{account_id}.r2.cloudflarestorage.com"),
            bucket,
            access_key_id,
            secret_access_key,
            folder,
        )
    }

    /// 支持自定义域名或预设 Endpoint 的 R2 构造器
    pub fn new_cloudflare_r2_with_endpoint(
        endpoint: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        folder: &str,
    ) -> Self {
        let endpoint_url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.trim_end_matches('/').to_string()
        } else {
            format!("https://{}", endpoint.trim_end_matches('/'))
        };
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: endpoint_url,
                region: "auto".to_string(),
                bucket: bucket.to_string(),
                path_style: true,
            }),
            segment_bytes: Some(512 * 1024), // 512KB 黄金平衡分片 (避免小分片导致的 API 往返与签名开销膨胀)
            poll_interval_ms: Some(30),      // 30ms 稳健轮询 (匹配边缘网络常规延迟，避免空轮询风暴)
            max_poll_interval_ms: Some(400), // 400ms 最大轮询上限 (空闲合理退避，保护配额)
            flush_interval_ms: Some(30),     // 30ms 快速刷出 (避免高频生成几字节碎片对象)
            eager_window_ms: Some(1500),     // 1.5s 积极轮询窗口 (稳定持续传输)
            concurrency: Some(16),           // 16 适度并发 (避免触发 R2 429 限流与 TCP 队头阻塞)
        }
    }

    /// 检查 S3 存储桶名是否适用于虚拟主机寻址 (Virtual-Hosted Style: `https://{bucket}.s3.{region}...`).
    ///
    /// 根据 S3 与 DNS 规范，虚拟主机寻址要求桶名必须是完全兼容 DNS 的：
    /// 1. 长度在 3 到 63 个字符之间；
    /// 2. 只能包含小写字母 (`a-z`)、数字 (`0-9`) 和连字符 (`-`)；
    /// 3. 必须以小写字母或数字开头和结尾；
    /// 4. 不含大写字母（包含大写字母时必须使用 Path-Style 路径寻址）；
    /// 5. 不含点 `.`（在 HTTPS 下通配符证书无法匹配包含点的多级子域名，AWS/B2 均要求此时使用 Path-Style）；
    /// 6. 不能是类似 IP 地址的格式。
    ///
    /// 若返回 `false`，则必须使用路径寻址 (`path_style = true`)，避免 Xray/S3 抛出：
    /// `transport/internet/xdrive: S3 virtual-hosted addressing requires a DNS-compatible bucket name`
    pub fn is_s3_virtual_hosted_compatible_bucket(bucket: &str) -> bool {
        let len = bucket.len();
        if !(3..=63).contains(&len) {
            return false;
        }
        let bytes = bucket.as_bytes();
        let is_alphanumeric_lower = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();

        // 必须以小写字母或数字开头和结尾
        if !is_alphanumeric_lower(bytes[0]) || !is_alphanumeric_lower(bytes[len - 1]) {
            return false;
        }

        // 字符必须全部为小写字母、数字或连字符（不包含大写字母、下划线、点号等）
        if !bytes.iter().all(|&b| is_alphanumeric_lower(b) || b == b'-') {
            return false;
        }

        true
    }

    /// 预设构建器：AWS S3 (Amazon Web Services)
    pub fn new_aws_s3(
        region: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        folder: &str,
    ) -> Self {
        let path_style = !Self::is_s3_virtual_hosted_compatible_bucket(bucket);
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: format!("https://s3.{region}.amazonaws.com"),
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

    /// 预设构建器：Oracle Cloud (甲骨文云) Object Storage
    pub fn new_oracle_cloud_s3(
        namespace: &str,
        region: &str,
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
                endpoint: format!(
                    "https://{namespace}.compat.objectstorage.{region}.oraclecloud.com"
                ),
                region: region.to_string(),
                bucket: bucket.to_string(),
                path_style: true,
            }),
            segment_bytes: Some(1024 * 1024),
            poll_interval_ms: Some(25),
            max_poll_interval_ms: Some(800),
            flush_interval_ms: None,
            eager_window_ms: None,
            concurrency: None,
        }
    }

    /// 预设构建器：Backblaze B2 (S3 兼容模式)
    pub fn new_backblaze_b2(
        region: &str,
        bucket: &str,
        key_id: &str,
        application_key: &str,
        folder: &str,
    ) -> Self {
        // Backblaze B2 允许大写字母桶名，但 S3 虚拟主机寻址 (virtual-hosted style)
        // 强制要求桶名符合 DNS 规范。若包含大写字母等非 DNS 字符，自适应切换为路径寻址 (path_style = true)。
        let path_style = !Self::is_s3_virtual_hosted_compatible_bucket(bucket);
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![key_id.to_string(), application_key.to_string()],
            s3: Some(S3Config {
                endpoint: format!("https://s3.{region}.backblazeb2.com"),
                region: region.to_string(),
                bucket: bucket.to_string(),
                path_style,
            }),
            segment_bytes: Some(1024 * 1024),
            poll_interval_ms: Some(1000), // 1000ms 保守防刷爆轮询（保护 B2 免费每日 2500 次限额）
            max_poll_interval_ms: Some(8000), // 8000ms 空闲深度退避
            flush_interval_ms: Some(200), // 200ms 批量写入
            eager_window_ms: Some(1000),  // 1s 积极轮询窗口
            concurrency: Some(2),         // 2 适度并发
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
        let effective_path_style =
            path_style || !Self::is_s3_virtual_hosted_compatible_bucket(bucket);
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: endpoint.to_string(),
                region: region.to_string(),
                bucket: bucket.to_string(),
                path_style: effective_path_style,
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

    /// 预设构建器：Microsoft OneDrive 个人版（自动装配公开凭证与经过验证的平衡流控参数）
    pub fn new_onedrive(refresh_token: &str, folder: &str) -> Self {
        Self::new_onedrive_custom(
            ONEDRIVE_DEFAULT_CLIENT_ID,
            ONEDRIVE_DEFAULT_CLIENT_SECRET,
            refresh_token,
            folder,
        )
    }

    /// 支持自定义凭证的高级构建器
    pub fn new_onedrive_custom(
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
        folder: &str,
    ) -> Self {
        Self {
            service: XdriveService::OneDrive,
            remote_folder: folder.trim_matches('/').to_string(),
            secrets: vec![
                client_id.to_string(),
                client_secret.to_string(),
                refresh_token.to_string(),
            ],
            s3: None,
            segment_bytes: Some(65536),       // 64KB 分片
            flush_interval_ms: Some(30),      // 30ms 快速刷出（降低往返缓冲等待时延）
            poll_interval_ms: Some(100), // 100ms 灵敏轮询（实测可提升响应速度约 1 秒且安全不触碰 429）
            max_poll_interval_ms: Some(1000), // 1000ms 空闲退避上限
            eager_window_ms: Some(2000), // 2s 积极轮询窗口
            concurrency: Some(4),        // 4 并发读写槽（防 429 限流保护）
        }
    }

    /// 生成完整的 Microsoft Graph API XDRIVE Template JSON 对象
    pub fn generate_onedrive_template_value(&self) -> serde_json::Value {
        serde_json::json!({
            "flatten": true,
            "concurrency": self.concurrency.unwrap_or(4),
            "auth": {
                "type": "oauth2",
                "tokenUrl": "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                "form": {
                    "client_id": "{secret0}",
                    "client_secret": "{secret1}",
                    "refresh_token": "{secret2}",
                    "grant_type": "refresh_token"
                },
                "tokenPath": "access_token",
                "expiryPath": "expires_in",
                "header": {
                    "Authorization": "Bearer {token}"
                }
            },
            "put": {
                "method": "PUT",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content",
                "headers": {
                    "Content-Type": "application/octet-stream"
                }
            },
            "get": {
                "method": "GET",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
            },
            "delete": {
                "method": "DELETE",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}"
            },
            "list": {
                "method": "GET",
                "url": "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}:/children?select=name",
                "namesRegex": "\"name\"\\s*:\\s*\"([^\"]+)\""
            },
            "retry": {
                "status": [429, 500, 502, 503, 504],
                "rateReason": "error.code"
            }
        })
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
            XdriveService::OneDrive => {
                if self.s3.is_some() {
                    bail!("OneDrive 服务模式不支持配置 s3 对象");
                }
                if self.secrets.is_empty() {
                    bail!("OneDrive 密钥参数 secrets 不能为空（至少包含 Refresh Token）");
                }
                if self.secrets.iter().any(|s| s.trim().is_empty()) {
                    bail!(
                        "OneDrive 凭据参数（Client ID / Client Secret / Refresh Token）均不能为空"
                    );
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

    /// 提取 Cloudflare 账户 ID 或自定义终结点/域名
    /// 支持纯 32 位十六进制 ID、完整终结点 URL 或自定义域名/Endpoint
    pub fn extract_cloudflare_account_id(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. 尝试从 https://<id>.r2.cloudflarestorage.com 或类似 URL 中提取 32 位 ID
        if let Some(pos) = trimmed.find(".r2.cloudflarestorage.com") {
            let prefix = &trimmed[..pos];
            let id = prefix.rsplit('/').next().unwrap_or(prefix);
            let id = id.rsplit(':').next().unwrap_or(id);
            if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(id.to_ascii_lowercase());
            }
        }

        // 2. 纯 32 位十六进制 ID
        if trimmed.len() == 32 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(trimmed.to_ascii_lowercase());
        }

        // 3. 自定义 Endpoint (http:// 或 https://)
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            if !trimmed.contains(char::is_whitespace) {
                let rest = trimmed
                    .trim_start_matches("https://")
                    .trim_start_matches("http://");
                let host = rest.split('/').next().unwrap_or("");
                let host = host.split(':').next().unwrap_or("");
                if !host.is_empty() && host.contains('.') {
                    return Some(trimmed.trim_end_matches('/').to_string());
                }
            }
            return None;
        }

        // 4. 自定义域名（如 r2.mycompany.com 或 my-bucket.example.org）
        if !trimmed.contains(char::is_whitespace)
            && !trimmed.contains('/')
            && trimmed.contains('.')
            && !trimmed.starts_with('.')
            && !trimmed.ends_with('.')
        {
            let (host, port) = trimmed.split_once(':').unwrap_or((trimmed, ""));
            let valid_port = port.is_empty() || port.chars().all(|c| c.is_ascii_digit());
            if valid_port {
                let parts: Vec<&str> = host.split('.').collect();
                let all_valid = parts.iter().all(|p| {
                    !p.is_empty()
                        && !p.starts_with('-')
                        && !p.ends_with('-')
                        && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                });
                if all_valid && parts.len() >= 2 {
                    return Some(trimmed.to_string());
                }
            }
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

    /// 智能解析 AWS S3 凭据全文（支持 credentials.ini、CSV、JSON、URL 与分步文本）
    /// 返回 (region, access_key_id, secret_access_key, bucket)
    pub fn extract_aws_s3_tokens(text: &str) -> Option<(String, String, String, Option<String>)> {
        let mut ak = None;
        let mut sk = None;
        let mut region = None;
        let mut bucket = None;

        let clean_val = |raw: &str| -> String {
            raw.trim()
                .trim_matches(|c| {
                    c == '"'
                        || c == '\''
                        || c == ','
                        || c == ';'
                        || c == '['
                        || c == ']'
                        || c == '{'
                        || c == '}'
                })
                .trim()
                .to_string()
        };

        let parse_s3_url = |url_str: &str| -> (Option<String>, Option<String>) {
            let mut found_bucket = None;
            let mut found_region = None;

            let without_proto = if let Some(pos) = url_str.find("://") {
                &url_str[pos + 3..]
            } else {
                url_str
            };

            let (host, path) = match without_proto.find('/') {
                Some(idx) => (&without_proto[..idx], &without_proto[idx + 1..]),
                None => (without_proto, ""),
            };

            let host_parts: Vec<&str> = host.split('.').collect();
            if host.contains("amazonaws.com") {
                if host_parts.len() >= 5
                    && (host_parts[1] == "s3" || host_parts[1].starts_with("s3-"))
                {
                    // my-bucket.s3.region.amazonaws.com
                    found_bucket = Some(host_parts[0].to_string());
                    found_region = Some(host_parts[2].to_string());
                } else if host_parts.len() >= 4 && host_parts[0] == "s3" {
                    // s3.region.amazonaws.com
                    found_region = Some(host_parts[1].to_string());
                } else if host_parts.len() >= 4 && host_parts[1] == "s3" {
                    // my-bucket.s3.amazonaws.com
                    found_bucket = Some(host_parts[0].to_string());
                } else if host_parts.len() >= 3 && host_parts[0].starts_with("s3-") {
                    // s3-region.amazonaws.com
                    found_region = Some(host_parts[0]["s3-".len()..].to_string());
                }
            }

            if found_bucket.is_none() && !path.is_empty() {
                let first_path = path.split(['/', '?', '#']).next().unwrap_or("").trim();
                if !first_path.is_empty() {
                    found_bucket = Some(first_path.to_string());
                }
            }

            (found_region, found_bucket)
        };

        let is_aws_region = |val: &str| -> bool {
            let parts: Vec<&str> = val.split('-').collect();
            parts.len() == 3
                && !parts[0].is_empty()
                && parts[0].chars().all(|c| c.is_ascii_lowercase())
                && !parts[1].is_empty()
                && parts[1].chars().all(|c| c.is_ascii_lowercase())
                && !parts[2].is_empty()
                && parts[2].chars().all(|c| c.is_ascii_digit())
        };

        // 1. 尝试解析 JSON 格式
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            let search_obj = if let Some(credentials) = v.get("credentials") {
                credentials
            } else {
                &v
            };

            if let Some(ak_val) = search_obj
                .get("access_key_id")
                .or_else(|| search_obj.get("AccessKeyId"))
                .or_else(|| search_obj.get("aws_access_key_id"))
                .or_else(|| search_obj.get("accessKeyId"))
                .and_then(|s| s.as_str())
            {
                ak = Some(clean_val(ak_val));
            }
            if let Some(sk_val) = search_obj
                .get("secret_access_key")
                .or_else(|| search_obj.get("SecretAccessKey"))
                .or_else(|| search_obj.get("aws_secret_access_key"))
                .or_else(|| search_obj.get("secretAccessKey"))
                .and_then(|s| s.as_str())
            {
                sk = Some(clean_val(sk_val));
            }
            if let Some(reg_val) = search_obj
                .get("region")
                .or_else(|| search_obj.get("Region"))
                .or_else(|| search_obj.get("aws_region"))
                .and_then(|s| s.as_str())
            {
                region = Some(clean_val(reg_val));
            }
            if let Some(bkt_val) = search_obj
                .get("bucket")
                .or_else(|| search_obj.get("Bucket"))
                .or_else(|| search_obj.get("bucket_name"))
                .or_else(|| search_obj.get("s3_bucket"))
                .and_then(|s| s.as_str())
            {
                bucket = Some(clean_val(bkt_val));
            }
        }

        // 2. 逐行分析（Key-Value、标签下一行值以及特征嗅探）
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

            // Access Key ID
            if ak.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part) {
                    let cleaned = clean_val(v);
                    if (k.contains("access_key_id")
                        || k.contains("access key id")
                        || k.contains("accesskeyid")
                        || k.contains("aws_access_key")
                        || k.contains("access_key")
                        || k.contains("访问密钥")
                        || k == "ak")
                        && !cleaned.is_empty()
                    {
                        ak = Some(cleaned);
                    }
                }
                if ak.is_none()
                    && (lower == "access key id"
                        || lower == "access_key_id"
                        || lower == "aws_access_key_id"
                        || lower == "accesskeyid"
                        || lower == "访问密钥 id"
                        || lower == "访问密钥")
                    && let Some(next) = lines.get(i + 1)
                {
                    let cleaned = clean_val(next);
                    if !cleaned.is_empty() {
                        ak = Some(cleaned);
                    }
                }
            }

            // Secret Access Key
            if sk.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part) {
                    let cleaned = clean_val(v);
                    if (k.contains("secret_access_key")
                        || k.contains("secret access key")
                        || k.contains("secretaccesskey")
                        || k.contains("aws_secret")
                        || k.contains("secret_key")
                        || k.contains("机密访问密钥")
                        || k.contains("私有访问密钥")
                        || k == "sk")
                        && !cleaned.is_empty()
                    {
                        sk = Some(cleaned);
                    }
                }
                if sk.is_none()
                    && (lower == "secret access key"
                        || lower == "secret_access_key"
                        || lower == "aws_secret_access_key"
                        || lower == "secretaccesskey"
                        || lower == "机密访问密钥"
                        || lower == "私有访问密钥")
                    && let Some(next) = lines.get(i + 1)
                {
                    let cleaned = clean_val(next);
                    if !cleaned.is_empty() {
                        sk = Some(cleaned);
                    }
                }
            }

            // Region
            if region.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part) {
                    let cleaned = clean_val(v);
                    if (k.contains("region") || k.contains("区域")) && !cleaned.is_empty() {
                        region = Some(cleaned);
                    }
                }
                if region.is_none()
                    && (lower == "region" || lower == "区域" || lower == "aws_region")
                    && let Some(next) = lines.get(i + 1)
                {
                    let cleaned = clean_val(next);
                    if !cleaned.is_empty() {
                        region = Some(cleaned);
                    }
                }
            }

            // Bucket
            if bucket.is_none() {
                if let (Some(k), Some(v)) = (&key_part, val_part) {
                    let cleaned = clean_val(v);
                    if (k.contains("bucket") || k.contains("存储桶")) && !cleaned.is_empty() {
                        bucket = Some(cleaned);
                    }
                }
                if bucket.is_none()
                    && (lower == "bucket" || lower == "存储桶" || lower == "bucket_name")
                    && let Some(next) = lines.get(i + 1)
                {
                    let cleaned = clean_val(next);
                    if !cleaned.is_empty() {
                        bucket = Some(cleaned);
                    }
                }
            }

            // CSV 单元格嗅探
            if line.contains(',') {
                let parts: Vec<&str> = line.split(',').collect();
                for part in parts {
                    let cleaned = clean_val(part);
                    if ak.is_none()
                        && (cleaned.starts_with("AKIA") || cleaned.starts_with("ASIA"))
                        && cleaned.len() == 20
                        && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        ak = Some(cleaned.clone());
                    }
                    if sk.is_none()
                        && cleaned.len() == 40
                        && cleaned
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '+' || c == '=')
                    {
                        sk = Some(cleaned.clone());
                    }
                    if region.is_none() && is_aws_region(&cleaned) {
                        region = Some(cleaned.clone());
                    }
                }
            }

            // 单词与 URL 嗅探
            for word in line.split_whitespace() {
                let cleaned = clean_val(word);
                if ak.is_none()
                    && (cleaned.starts_with("AKIA") || cleaned.starts_with("ASIA"))
                    && cleaned.len() == 20
                    && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    ak = Some(cleaned.clone());
                }
                if sk.is_none()
                    && cleaned.len() == 40
                    && cleaned
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '+' || c == '=')
                    && !cleaned.chars().all(|c| c.is_ascii_digit())
                {
                    sk = Some(cleaned.clone());
                }
                if region.is_none() && is_aws_region(&cleaned) {
                    region = Some(cleaned.clone());
                }
                if cleaned.contains("amazonaws.com") {
                    let (r, b) = parse_s3_url(&cleaned);
                    if region.is_none()
                        && let Some(r_val) = r
                    {
                        region = Some(r_val);
                    }
                    if bucket.is_none()
                        && let Some(b_val) = b
                    {
                        bucket = Some(b_val);
                    }
                }
            }
        }

        match (ak, sk) {
            (Some(a), Some(s)) => {
                let a = clean_val(&a);
                let s = clean_val(&s);
                if !a.is_empty() && !s.is_empty() {
                    let r = region
                        .map(|r| clean_val(&r))
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "ap-northeast-1".to_string());
                    let b = bucket.map(|b| clean_val(&b)).filter(|b| !b.is_empty());
                    Some((r, a, s, b))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 智能解析 Oracle Cloud (甲骨文云) 凭据全文
    /// 返回 (namespace, region, access_key_id, secret_access_key, bucket)
    pub fn extract_oracle_cloud_tokens(
        text: &str,
    ) -> Option<(String, String, String, String, Option<String>)> {
        let mut ns = None;
        let mut region = None;
        let mut ak = None;
        let mut sk = None;
        let mut bucket = None;

        let clean_val = |raw: &str| -> String {
            raw.trim()
                .trim_matches(|c| {
                    c == '"'
                        || c == '\''
                        || c == ','
                        || c == ';'
                        || c == '['
                        || c == ']'
                        || c == '{'
                        || c == '}'
                })
                .trim()
                .to_string()
        };

        let parse_oci_url = |url_str: &str| -> (Option<String>, Option<String>, Option<String>) {
            let mut found_ns = None;
            let mut found_region = None;
            let mut found_bucket = None;

            let without_proto = if let Some(pos) = url_str.find("://") {
                &url_str[pos + 3..]
            } else {
                url_str
            };

            let (host, path) = match without_proto.find('/') {
                Some(idx) => (&without_proto[..idx], &without_proto[idx + 1..]),
                None => (without_proto, ""),
            };

            if host.contains("oraclecloud.com") && host.contains("compat.objectstorage") {
                let parts: Vec<&str> = host.split('.').collect();
                if parts.len() >= 5 {
                    found_ns = Some(parts[0].to_string());
                    found_region = Some(parts[3].to_string());
                }
            }

            if !path.is_empty() {
                let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                if let Some(pos) = segments.iter().position(|&s| s == "bucket") {
                    if pos + 1 < segments.len() {
                        found_bucket = Some(segments[pos + 1].to_string());
                    }
                } else if !segments.is_empty() && !segments[0].starts_with("v1") {
                    found_bucket = Some(segments[0].to_string());
                }
            }

            (found_ns, found_region, found_bucket)
        };

        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        for line in &lines {
            let (key_part, val_part) = if let Some(pos) = line.find(['=', ':']) {
                let k = line[..pos].trim().to_ascii_lowercase();
                let v = line[pos + 1..].trim();
                (Some(k), Some(v))
            } else {
                (None, None)
            };

            if let (Some(k), Some(v)) = (&key_part, val_part) {
                let cleaned = clean_val(v);
                if (k.contains("namespace") || k == "ns") && !cleaned.is_empty() {
                    ns = Some(cleaned.clone());
                } else if (k.contains("region") || k == "location") && !cleaned.is_empty() {
                    region = Some(cleaned.clone());
                } else if (k.contains("secret_key")
                    || k.contains("secret_access_key")
                    || k.contains("secretaccesskey")
                    || k.contains("secret key")
                    || k == "sk")
                    && !cleaned.is_empty()
                {
                    sk = Some(cleaned.clone());
                } else if (k.contains("access_key")
                    || k.contains("access_key_id")
                    || k.contains("accesskeyid")
                    || k.contains("access key")
                    || k == "ak")
                    && !cleaned.is_empty()
                {
                    ak = Some(cleaned.clone());
                } else if k.contains("bucket") && !cleaned.is_empty() {
                    bucket = Some(cleaned.clone());
                }
            }

            for word in line.split_whitespace() {
                let cleaned = clean_val(word);
                if cleaned.contains("compat.objectstorage") && cleaned.contains("oraclecloud.com") {
                    let (n_val, r_val, b_val) = parse_oci_url(&cleaned);
                    if ns.is_none()
                        && let Some(n) = n_val
                    {
                        ns = Some(n);
                    }
                    if region.is_none()
                        && let Some(r) = r_val
                    {
                        region = Some(r);
                    }
                    if bucket.is_none()
                        && let Some(b) = b_val
                    {
                        bucket = Some(b);
                    }
                }
            }
        }

        match (ns, ak, sk) {
            (Some(n), Some(a), Some(s)) => {
                let n = clean_val(&n);
                let a = clean_val(&a);
                let s = clean_val(&s);
                if !n.is_empty() && !a.is_empty() && !s.is_empty() {
                    let r = region
                        .map(|r| clean_val(&r))
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "ap-chuncheon-1".to_string());
                    let b = bucket.map(|b| clean_val(&b)).filter(|b| !b.is_empty());
                    Some((n, r, a, s, b))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 智能解析 Backblaze B2 凭据全文
    /// 返回 (region, key_id, application_key, bucket)
    pub fn extract_backblaze_b2_tokens(
        text: &str,
    ) -> Option<(String, String, String, Option<String>)> {
        let mut key_id = None;
        let mut app_key = None;
        let mut region = None;
        let mut bucket = None;

        let clean_val = |raw: &str| -> String {
            raw.trim()
                .trim_matches(|c| {
                    c == '"'
                        || c == '\''
                        || c == ','
                        || c == ';'
                        || c == '['
                        || c == ']'
                        || c == '{'
                        || c == '}'
                })
                .trim()
                .to_string()
        };

        let parse_b2_url = |url_str: &str| -> Option<String> {
            let without_proto = if let Some(pos) = url_str.find("://") {
                &url_str[pos + 3..]
            } else {
                url_str
            };

            let host = match without_proto.find('/') {
                Some(idx) => &without_proto[..idx],
                None => without_proto,
            };

            if host.contains("backblazeb2.com") {
                let parts: Vec<&str> = host.split('.').collect();
                if parts.len() >= 4 && parts[0] == "s3" {
                    return Some(parts[1].to_string());
                } else if parts.len() >= 5 && parts[1] == "s3" {
                    return Some(parts[2].to_string());
                }
            }
            None
        };

        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        for line in &lines {
            let (key_part, val_part) = if let Some(pos) = line.find(['=', ':']) {
                let k = line[..pos].trim().to_ascii_lowercase();
                let v = line[pos + 1..].trim();
                (Some(k), Some(v))
            } else {
                (None, None)
            };

            if let (Some(k), Some(v)) = (&key_part, val_part) {
                let cleaned = clean_val(v);
                if (k.contains("keyid") || k.contains("key_id") || k.contains("key id"))
                    && !cleaned.is_empty()
                {
                    key_id = Some(cleaned.clone());
                } else if (k.contains("applicationkey")
                    || k.contains("application_key")
                    || k.contains("application key")
                    || k.contains("appkey")
                    || k.contains("app_key")
                    || k.contains("app key"))
                    && !cleaned.is_empty()
                {
                    app_key = Some(cleaned.clone());
                } else if k.contains("region") && !cleaned.is_empty() {
                    region = Some(cleaned.clone());
                } else if k.contains("bucket") && !cleaned.is_empty() {
                    bucket = Some(cleaned.clone());
                }
            }

            for word in line.split_whitespace() {
                let cleaned = clean_val(word);
                if cleaned.contains("backblazeb2.com")
                    && region.is_none()
                    && let Some(r) = parse_b2_url(&cleaned)
                {
                    region = Some(r);
                }
                if key_id.is_none()
                    && cleaned.len() == 25
                    && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
                    && cleaned.starts_with("00")
                {
                    key_id = Some(cleaned.clone());
                }
                if app_key.is_none()
                    && cleaned.len() == 31
                    && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
                    && cleaned.starts_with('K')
                {
                    app_key = Some(cleaned.clone());
                }
            }
        }

        match (key_id, app_key) {
            (Some(k), Some(a)) => {
                let k = clean_val(&k);
                let a = clean_val(&a);
                if !k.is_empty() && !a.is_empty() {
                    let r = region
                        .map(|r| clean_val(&r))
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "us-west-004".to_string());
                    let b = bucket.map(|b| clean_val(&b)).filter(|b| !b.is_empty());
                    Some((r, k, a, b))
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
        assert_eq!(config.poll_interval_ms, Some(30));
        assert_eq!(config.segment_bytes, Some(512 * 1024));
        assert_eq!(config.max_poll_interval_ms, Some(400));
        assert_eq!(config.flush_interval_ms, Some(30));
        assert_eq!(config.eager_window_ms, Some(1500));
        assert_eq!(config.concurrency, Some(16));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cloudflare_r2_optimized_parameters() {
        let config = XdriveConfig::new_cloudflare_r2(
            "testaccount123456789012345678901",
            "my-bucket",
            "my-ak",
            "my-sk",
            "aegis-xdrive",
        );
        assert_eq!(config.segment_bytes, Some(524288));
        assert_eq!(config.concurrency, Some(16));
        assert_eq!(config.poll_interval_ms, Some(30));
        assert_eq!(config.max_poll_interval_ms, Some(400));
        assert_eq!(config.flush_interval_ms, Some(30));
        assert_eq!(config.eager_window_ms, Some(1500));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cloudflare_r2_with_custom_endpoint() {
        let config_url = XdriveConfig::new_cloudflare_r2_with_endpoint(
            "https://r2.mycompany.com/",
            "my-bucket",
            "my-ak",
            "my-sk",
            "aegis-xdrive",
        );
        assert_eq!(config_url.service, XdriveService::S3);
        let s3 = config_url.s3.as_ref().expect("s3 config must be present");
        assert_eq!(s3.endpoint, "https://r2.mycompany.com");
        assert_eq!(s3.bucket, "my-bucket");
        assert_eq!(s3.region, "auto");
        assert!(s3.path_style);
        assert_eq!(config_url.segment_bytes, Some(524288));
        assert_eq!(config_url.concurrency, Some(16));
        assert_eq!(config_url.poll_interval_ms, Some(30));
        assert_eq!(config_url.max_poll_interval_ms, Some(400));
        assert_eq!(config_url.flush_interval_ms, Some(30));
        assert_eq!(config_url.eager_window_ms, Some(1500));
        assert!(config_url.validate().is_ok());

        // Test with raw domain (without https://)
        let config_domain = XdriveConfig::new_cloudflare_r2_with_endpoint(
            "r2.mycompany.com",
            "my-bucket",
            "my-ak",
            "my-sk",
            "aegis-xdrive",
        );
        assert_eq!(
            config_domain.s3.as_ref().unwrap().endpoint,
            "https://r2.mycompany.com"
        );
        assert!(config_domain.validate().is_ok());
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

        // Custom endpoints & domains
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("https://r2.mycompany.com"),
            Some("https://r2.mycompany.com".to_string())
        );
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("https://r2.mycompany.com/"),
            Some("https://r2.mycompany.com".to_string())
        );
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("r2.mycompany.com"),
            Some("r2.mycompany.com".to_string())
        );
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("custom-r2.storage.cloudflare.net:8443"),
            Some("custom-r2.storage.cloudflare.net:8443".to_string())
        );

        assert_eq!(XdriveConfig::extract_cloudflare_account_id("invalid"), None);
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("ftp://invalid.com"),
            None
        );
        assert_eq!(
            XdriveConfig::extract_cloudflare_account_id("not a domain"),
            None
        );
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
    fn test_valid_onedrive_preset() {
        let config = XdriveConfig::new_onedrive("refresh-token-xyz", "/xray_tunnel");

        assert_eq!(config.service, XdriveService::OneDrive);
        assert_eq!(config.remote_folder, "xray_tunnel");
        assert_eq!(
            config.secrets,
            vec![
                ONEDRIVE_DEFAULT_CLIENT_ID.to_string(),
                ONEDRIVE_DEFAULT_CLIENT_SECRET.to_string(),
                "refresh-token-xyz".to_string()
            ]
        );
        assert!(config.s3.is_none());
        assert_eq!(config.segment_bytes, Some(65536));
        assert_eq!(config.flush_interval_ms, Some(30));
        assert_eq!(config.poll_interval_ms, Some(100));
        assert_eq!(config.max_poll_interval_ms, Some(1000));
        assert_eq!(config.eager_window_ms, Some(2000));
        assert_eq!(config.concurrency, Some(4));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_valid_onedrive_custom() {
        let config = XdriveConfig::new_onedrive_custom(
            "custom-client-id",
            "custom-client-secret",
            "refresh-token-xyz",
            "/xray_tunnel/",
        );

        assert_eq!(config.service, XdriveService::OneDrive);
        assert_eq!(config.remote_folder, "xray_tunnel");
        assert_eq!(
            config.secrets,
            vec![
                "custom-client-id".to_string(),
                "custom-client-secret".to_string(),
                "refresh-token-xyz".to_string(),
            ]
        );
        assert!(config.s3.is_none());
        assert_eq!(config.segment_bytes, Some(65536));
        assert_eq!(config.flush_interval_ms, Some(30));
        assert_eq!(config.poll_interval_ms, Some(100));
        assert_eq!(config.max_poll_interval_ms, Some(1000));
        assert_eq!(config.eager_window_ms, Some(2000));
        assert_eq!(config.concurrency, Some(4));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_onedrive_template_generation() {
        let config = XdriveConfig::new_onedrive("refresh-token-xyz", "my_tunnel");
        let template = config.generate_onedrive_template_value();

        assert_eq!(template["flatten"], true);
        assert_eq!(template["concurrency"], 4);
        assert_eq!(template["auth"]["type"], "oauth2");
        assert_eq!(
            template["auth"]["tokenUrl"],
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        assert_eq!(template["auth"]["form"]["client_id"], "{secret0}");
        assert_eq!(template["auth"]["form"]["client_secret"], "{secret1}");
        assert_eq!(template["auth"]["form"]["refresh_token"], "{secret2}");
        assert_eq!(template["auth"]["form"]["grant_type"], "refresh_token");
        assert_eq!(template["auth"]["tokenPath"], "access_token");
        assert_eq!(template["auth"]["expiryPath"], "expires_in");
        assert_eq!(
            template["auth"]["header"]["Authorization"],
            "Bearer {token}"
        );

        assert_eq!(template["put"]["method"], "PUT");
        assert_eq!(
            template["put"]["url"],
            "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
        );
        assert_eq!(
            template["put"]["headers"]["Content-Type"],
            "application/octet-stream"
        );

        assert_eq!(template["get"]["method"], "GET");
        assert_eq!(
            template["get"]["url"],
            "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
        );

        assert_eq!(template["delete"]["method"], "DELETE");
        assert_eq!(
            template["delete"]["url"],
            "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}"
        );

        assert_eq!(template["list"]["method"], "GET");
        assert_eq!(
            template["list"]["url"],
            "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}:/children?select=name"
        );
        assert_eq!(
            template["list"]["namesRegex"],
            "\"name\"\\s*:\\s*\"([^\"]+)\""
        );

        assert_eq!(
            template["retry"]["status"],
            serde_json::json!([429, 500, 502, 503, 504])
        );
        assert_eq!(template["retry"]["rateReason"], "error.code");

        // 测试并发覆盖
        let mut custom_concurrency_config = config.clone();
        custom_concurrency_config.concurrency = Some(16);
        let custom_template = custom_concurrency_config.generate_onedrive_template_value();
        assert_eq!(custom_template["concurrency"], 16);
    }

    #[test]
    fn test_onedrive_validation_rules() {
        // 正常情况：包含 refresh_token 即可通过
        let mut config = XdriveConfig::new_onedrive("refresh", "/tunnel");
        assert!(config.validate().is_ok());

        // 单凭据（仅含有 refresh_token）允许通过
        config.secrets = vec!["refresh-token-only".to_string()];
        assert!(config.validate().is_ok());

        // 空 secrets 列表拒绝
        config.secrets = vec![];
        assert!(config.validate().is_err());

        // 包含空/纯空白凭据拒绝
        config.secrets = vec!["cid".to_string(), "   ".to_string(), "refresh".to_string()];
        assert!(config.validate().is_err());

        let mut config2 = XdriveConfig::new_onedrive("refresh", "/tunnel");
        config2.secrets[0] = "".to_string();
        assert!(config2.validate().is_err());

        // remote_folder 为空拒绝
        let config3 = XdriveConfig::new_onedrive("refresh", "   ");
        assert!(config3.validate().is_err());

        let config4 = XdriveConfig::new_onedrive("refresh", "///");
        assert!(config4.validate().is_err());

        // 配置了 S3 对象拒绝
        let mut config5 = XdriveConfig::new_onedrive("refresh", "/tunnel");
        config5.s3 = Some(S3Config {
            endpoint: "https://example.com".to_string(),
            region: "auto".to_string(),
            bucket: "bucket".to_string(),
            path_style: true,
        });
        assert!(config5.validate().is_err());
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

    #[test]
    fn test_valid_aws_s3_preset() {
        let config = XdriveConfig::new_aws_s3(
            "ap-northeast-1",
            "my-aws-bucket",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "aegis-xdrive",
        );
        assert_eq!(config.service, XdriveService::S3);
        assert_eq!(config.remote_folder, "aegis-xdrive");
        assert_eq!(
            config.secrets,
            vec![
                "AKIAIOSFODNN7EXAMPLE".to_string(),
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()
            ]
        );
        let s3 = config.s3.as_ref().expect("s3 config must be present");
        assert_eq!(s3.endpoint, "https://s3.ap-northeast-1.amazonaws.com");
        assert_eq!(s3.region, "ap-northeast-1");
        assert_eq!(s3.bucket, "my-aws-bucket");
        assert!(!s3.path_style);
        assert_eq!(config.segment_bytes, Some(1024 * 1024));
        assert_eq!(config.poll_interval_ms, Some(25));
        assert_eq!(config.max_poll_interval_ms, Some(800));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_extract_aws_s3_tokens() {
        // 1. AWS credentials ini 格式
        let text_ini = r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
region = us-east-1
"#;
        let (region, ak, sk, bucket) =
            XdriveConfig::extract_aws_s3_tokens(text_ini).expect("INI 凭据解析失败");
        assert_eq!(region, "us-east-1");
        assert_eq!(ak, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(sk, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(bucket, None);

        // 2. CSV 格式（带默认区域 ap-northeast-1）
        let text_csv = r#"User Name,Access key ID,Secret access key
admin,AKIAI44QH8DHBEXAMPLE,je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY"#;
        let (region, ak, sk, bucket) =
            XdriveConfig::extract_aws_s3_tokens(text_csv).expect("CSV 凭据解析失败");
        assert_eq!(region, "ap-northeast-1");
        assert_eq!(ak, "AKIAI44QH8DHBEXAMPLE");
        assert_eq!(sk, "je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY");
        assert_eq!(bucket, None);

        // 3. JSON 格式（带 bucket）
        let text_json = r#"{
            "access_key_id": "AKIAIOSFODNN7EXAMPLE",
            "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "region": "ap-southeast-1",
            "bucket": "test-s3-bucket"
        }"#;
        let (region, ak, sk, bucket) =
            XdriveConfig::extract_aws_s3_tokens(text_json).expect("JSON 凭据解析失败");
        assert_eq!(region, "ap-southeast-1");
        assert_eq!(ak, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(sk, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(bucket, Some("test-s3-bucket".to_string()));

        // 4. URL 与 endpoint 文本嗅探
        let text_url = "endpoint: https://my-s3-bucket.s3.eu-central-1.amazonaws.com\nAKIAIOSFODNN7EXAMPLE\nwJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let (region, ak, sk, bucket) =
            XdriveConfig::extract_aws_s3_tokens(text_url).expect("URL 凭据解析失败");
        assert_eq!(region, "eu-central-1");
        assert_eq!(ak, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(sk, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(bucket, Some("my-s3-bucket".to_string()));

        // 5. 缺失必需凭据应返回 None
        let text_missing = "region = us-west-2\naws_access_key_id = AKIAIOSFODNN7EXAMPLE\n";
        assert!(XdriveConfig::extract_aws_s3_tokens(text_missing).is_none());
    }

    #[test]
    fn test_new_oracle_cloud_s3() {
        let config = XdriveConfig::new_oracle_cloud_s3(
            "ax1234567890",
            "ap-chuncheon-1",
            "my-oracle-bucket",
            "my_access_key",
            "my_secret_key",
            "xray-folder",
        );
        assert_eq!(config.service, XdriveService::S3);
        assert_eq!(config.remote_folder, "xray-folder");
        assert_eq!(config.secrets, vec!["my_access_key", "my_secret_key"]);
        let s3 = config.s3.expect("s3 config must be set");
        assert_eq!(
            s3.endpoint,
            "https://ax1234567890.compat.objectstorage.ap-chuncheon-1.oraclecloud.com"
        );
        assert_eq!(s3.region, "ap-chuncheon-1");
        assert_eq!(s3.bucket, "my-oracle-bucket");
        assert!(s3.path_style);
    }

    #[test]
    fn test_new_backblaze_b2() {
        let config = XdriveConfig::new_backblaze_b2(
            "us-west-004",
            "my-b2-bucket",
            "004a1b2c3d4e5f60000000001",
            "K004abcdef1234567890abcdef12345",
            "b2-folder",
        );
        assert_eq!(config.service, XdriveService::S3);
        assert_eq!(config.remote_folder, "b2-folder");
        assert_eq!(
            config.secrets,
            vec![
                "004a1b2c3d4e5f60000000001",
                "K004abcdef1234567890abcdef12345"
            ]
        );
        let s3 = config.s3.expect("s3 config must be set");
        assert_eq!(s3.endpoint, "https://s3.us-west-004.backblazeb2.com");
        assert_eq!(s3.region, "us-west-004");
        assert_eq!(s3.bucket, "my-b2-bucket");
        assert!(!s3.path_style);
    }

    #[test]
    fn test_new_backblaze_b2_uppercase_bucket() {
        let config = XdriveConfig::new_backblaze_b2(
            "ca-east-006",
            "mytest1vIvHY",
            "004a1b2c3d4e5f60000000001",
            "K004abcdef1234567890abcdef12345",
            "b2-folder",
        );
        let s3 = config.s3.expect("s3 config must be set");
        assert_eq!(s3.endpoint, "https://s3.ca-east-006.backblazeb2.com");
        assert_eq!(s3.region, "ca-east-006");
        assert_eq!(s3.bucket, "mytest1vIvHY");
        assert!(
            s3.path_style,
            "Bucket with uppercase letters must automatically fallback to path_style = true"
        );
    }

    #[test]
    fn test_is_s3_virtual_hosted_compatible_bucket() {
        assert!(XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "my-b2-bucket"
        ));
        assert!(XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "bucket123"
        ));
        assert!(XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "123bucket"
        ));

        // 包含大写字母
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "mytest1vIvHY"
        ));
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "Bucket"
        ));

        // 下划线
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "my_bucket"
        ));

        // 点（多级子域名在 HTTPS 通配符证书下无法匹配）
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "my.bucket"
        ));

        // 连字符在两端
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "-bucket"
        ));
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            "bucket-"
        ));

        // 长度边界
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket("ab"));
        assert!(!XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            &"a".repeat(64)
        ));
        assert!(XdriveConfig::is_s3_virtual_hosted_compatible_bucket(
            &"a".repeat(63)
        ));
    }

    #[test]
    fn test_extract_oracle_cloud_tokens() {
        let text_ini = r#"
[DEFAULT]
user=ocid1.user.oc1..aaaaaaaaxxx
fingerprint=aa:bb:cc:dd
key_file=~/.oci/oci_api_key.pem
tenancy=ocid1.tenancy.oc1..aaaaaaaayyy
region=ap-chuncheon-1
namespace=ax1234567890
access_key_id=my_oci_access_key
secret_access_key=my_oci_secret_key
bucket=my-backup-bucket
"#;
        let (ns, region, ak, sk, bucket) =
            XdriveConfig::extract_oracle_cloud_tokens(text_ini).expect("OCI parse failed");
        assert_eq!(ns, "ax1234567890");
        assert_eq!(region, "ap-chuncheon-1");
        assert_eq!(ak, "my_oci_access_key");
        assert_eq!(sk, "my_oci_secret_key");
        assert_eq!(bucket, Some("my-backup-bucket".to_string()));

        // Also test endpoint URL parsing
        let text_url = "Endpoint: https://mytestns.compat.objectstorage.ap-tokyo-1.oraclecloud.com/v1/bucket/mybucket\nAK: test_ak\nSK: test_sk";
        let (ns, region, ak, sk, bucket) =
            XdriveConfig::extract_oracle_cloud_tokens(text_url).expect("OCI URL parse failed");
        assert_eq!(ns, "mytestns");
        assert_eq!(region, "ap-tokyo-1");
        assert_eq!(ak, "test_ak");
        assert_eq!(sk, "test_sk");
        assert_eq!(bucket, Some("mybucket".to_string()));
    }

    #[test]
    fn test_extract_backblaze_b2_tokens() {
        let text = "keyID: 004a1b2c3d4e5f60000000001\napplicationKey: K004abcdef1234567890abcdef12345\nEndpoint: s3.us-west-004.backblazeb2.com\nBucket: my-b2-vault";
        let (region, key_id, app_key, bucket) =
            XdriveConfig::extract_backblaze_b2_tokens(text).expect("B2 parse failed");
        assert_eq!(region, "us-west-004");
        assert_eq!(key_id, "004a1b2c3d4e5f60000000001");
        assert_eq!(app_key, "K004abcdef1234567890abcdef12345");
        assert_eq!(bucket, Some("my-b2-vault".to_string()));
    }
}
