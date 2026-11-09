use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::time::Duration;

pub static SHA256_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)sha256[:\s]+([0-9a-f]{64})").expect("valid sha256 regex"));

const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

#[derive(Debug, Deserialize)]
pub struct ReleaseResponse {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
    #[serde(default)]
    pub prerelease: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub url: String,
    pub size: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
}

impl ReleaseAsset {
    pub fn download_url(&self) -> &str {
        if !self.browser_download_url.is_empty() {
            return &self.browser_download_url;
        }
        if !self.url.is_empty() {
            return &self.url;
        }
        ""
    }
}

pub async fn fetch_json_from_mirrors<T: DeserializeOwned>(
    client: &reqwest::Client,
    bases: &[String],
    api_path: &str,
    token: Option<&str>,
) -> Result<T> {
    let mut last_err = None::<anyhow::Error>;
    for base in bases {
        let url = format!(
            "{}/{}",
            base.trim_end_matches('/'),
            api_path.trim_start_matches('/')
        );
        let mut builder = client
            .get(&url)
            .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
            .header(
                ACCEPT,
                HeaderValue::from_static("application/vnd.github+json"),
            );
        if let Some(t) = token {
            builder = builder.bearer_auth(t);
        }
        let response = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        };
        let response = match response.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        };
        match response.json::<T>().await {
            Ok(data) => return Ok(data),
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("所有镜像源均失败")))
}

pub async fn fetch_prerelease(
    client: &reqwest::Client,
    bases: &[String],
    api_path: &str,
    token: Option<&str>,
) -> Result<ReleaseResponse> {
    let releases: Vec<ReleaseResponse> =
        fetch_json_from_mirrors(client, bases, api_path, token).await?;
    releases
        .into_iter()
        .find(|r| r.prerelease)
        .ok_or_else(|| anyhow!("No prerelease found"))
}

/// 取 GitHub 返回列表中的**最新版**（列表已按发布时间倒序）。
///
/// 供 Xray-core 使用，**语义与 `fetch_prerelease` 不同且不可互相替代**：
/// 上游 XTLS/Xray-core 平时发布 `prerelease: true`，偶尔把某个版本标为正式版。
/// 若像 sing-box 那样「只要 prerelease」，就会跳过更新的正式版
/// （实测：v26.10.21 是正式版、v26.9.30 仍是 prerelease，按 prerelease
/// 过滤会装到旧的 v26.9.30）。Xray 侧要的是「最新」，与是否预发行无关。
///
/// sing-box 仍走 `fetch_prerelease`，两条发行通道必须保持独立。
pub fn pick_latest_release(releases: Vec<ReleaseResponse>) -> Result<ReleaseResponse> {
    releases
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No release found"))
}

pub fn parse_digest(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    lower
        .strip_prefix("sha256:")
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() == 64)
}

pub const DEFAULT_DOWNLOAD_MAX_RETRIES: usize = 4;
pub const DEFAULT_DOWNLOAD_INITIAL_DELAY: Duration = Duration::from_secs(2);

/// 判断 HTTP 状态码是否属于可瞬态重试错误（5xx, 408, 429）。
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// 判断 reqwest::Error 是否属于可重试的临时网络错误。
pub fn is_retryable_reqwest_error(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() {
        return true;
    }
    if let Some(status) = err.status() {
        return is_retryable_status(status);
    }
    err.is_request()
}

/// 判断 anyhow::Error 是否可瞬态重试（检查下转的 reqwest::Error 或特定错误文案）。
pub fn is_retryable_anyhow_error(err: &anyhow::Error) -> bool {
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>() {
        return is_retryable_reqwest_error(req_err);
    }
    // 递归检查 source chain 是否包含 reqwest::Error
    for cause in err.chain() {
        if let Some(req_err) = cause.downcast_ref::<reqwest::Error>() {
            return is_retryable_reqwest_error(req_err);
        }
    }
    let msg = err.to_string().to_ascii_lowercase();

    // 状态码文本必须具备明确的 HTTP 边界，避免端口号（如 35048）或 commit 哈希包含 504 时被误伤
    let has_retryable_status = msg.contains("(504")
        || msg.contains("(502")
        || msg.contains("(503")
        || msg.contains("(408")
        || msg.contains("(429")
        || msg.contains(" 504 ")
        || msg.contains(" 502 ")
        || msg.contains(" 503 ")
        || msg.contains("status code: 504")
        || msg.contains("status code: 502")
        || msg.contains("status code: 503");

    has_retryable_status
        || msg.contains("gateway timeout")
        || msg.contains("bad gateway")
        || msg.contains("service unavailable")
        || msg.contains("connection reset")
        || msg.contains("connection closed")
        || msg.contains("timed out")
        || msg.contains("timeout")
}

/// 针对异步操作执行指数退避重试。
pub async fn retry_with_backoff<T, F, Fut>(
    operation_name: &str,
    max_retries: usize,
    initial_delay: Duration,
    mut op: F,
) -> Result<T>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delay = initial_delay;
    for attempt in 1..=max_retries {
        match op(attempt).await {
            Ok(val) => return Ok(val),
            Err(err) => {
                if attempt == max_retries || !is_retryable_anyhow_error(&err) {
                    return Err(err);
                }
                log::warn!(
                    "{operation_name} (第 {attempt}/{max_retries} 次) 遇到瞬态错误: {err:#}，将在 {delay:?} 后重试..."
                );
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
        }
    }
    anyhow::bail!("{operation_name} 重试次数耗尽")
}

pub fn parse_sha256_manifest(manifest: &str, target_asset: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let filename = parts.next().unwrap_or("");
        if (filename.ends_with(target_asset) || filename == target_asset) && hash.len() == 64 {
            return Some(hash.to_string());
        }
    }
    None
}

pub fn extract_sha256_from_body(body: &str) -> Option<String> {
    SHA256_LINE_RE
        .captures(body)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

pub fn find_minisig_asset<'a>(
    assets: &'a [ReleaseAsset],
    binary_name: &str,
) -> Option<&'a ReleaseAsset> {
    let sig_name = format!("{}.minisig", binary_name);
    assets.iter().find(|a| a.name == sig_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_digest_valid_and_invalid() {
        assert_eq!(
            parse_digest("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
        );
        assert!(parse_digest("md5:abcd").is_none());
        assert!(parse_digest("sha256:1234").is_none());
    }

    #[test]
    fn test_parse_sha256_manifest() {
        let manifest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  aegis\n";
        let result = parse_sha256_manifest(manifest, "aegis");
        assert_eq!(
            result,
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
        );
        assert!(parse_sha256_manifest("", "aegis").is_none());
    }

    #[test]
    fn test_extract_sha256_from_body() {
        let body = "Release notes\nSHA256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = extract_sha256_from_body(body);
        assert!(result.is_some());
        assert!(extract_sha256_from_body("no hash here").is_none());
    }

    #[test]
    fn test_release_asset_download_url_prefers_browser_download_url() {
        let asset = ReleaseAsset {
            name: "test.zip".to_string(),
            browser_download_url: "https://example.com/download".to_string(),
            url: "https://api.example.com/assets/1".to_string(),
            size: None,
            digest: None,
        };
        assert_eq!(asset.download_url(), "https://example.com/download");
    }

    #[test]
    fn test_release_asset_download_url_falls_back_to_url() {
        let asset = ReleaseAsset {
            name: "test.zip".to_string(),
            browser_download_url: "".to_string(),
            url: "https://api.example.com/assets/1".to_string(),
            size: None,
            digest: None,
        };
        assert_eq!(asset.download_url(), "https://api.example.com/assets/1");
    }

    #[test]
    fn test_release_asset_download_url_returns_empty_when_both_missing() {
        let asset = ReleaseAsset {
            name: "test.zip".to_string(),
            browser_download_url: "".to_string(),
            url: "".to_string(),
            size: None,
            digest: None,
        };
        assert_eq!(asset.download_url(), "");
    }

    fn release(tag: &str, prerelease: bool) -> ReleaseResponse {
        ReleaseResponse {
            tag_name: tag.to_string(),
            body: None,
            assets: vec![],
            prerelease,
        }
    }

    // 回归：Xray 上游把最新版标为正式版、旧版仍标 prerelease 时，
    // 必须拿到最新那条，而不是第一个 prerelease。
    #[test]
    fn test_pick_latest_release_prefers_newest_not_prerelease() {
        let picked = pick_latest_release(vec![
            release("v26.10.21", false),
            release("v26.9.30", true),
            release("v26.9.9", true),
        ])
        .expect("should pick");
        assert_eq!(picked.tag_name, "v26.10.21");
    }

    #[test]
    fn test_pick_latest_release_errors_on_empty() {
        assert!(pick_latest_release(vec![]).is_err());
    }

    #[test]
    fn test_find_minisig_asset_found() {
        let assets = vec![
            ReleaseAsset {
                name: "aegis".to_string(),
                browser_download_url: "https://example.com/aegis".to_string(),
                url: String::new(),
                size: None,
                digest: None,
            },
            ReleaseAsset {
                name: "aegis.minisig".to_string(),
                browser_download_url: "https://example.com/aegis.minisig".to_string(),
                url: String::new(),
                size: None,
                digest: None,
            },
        ];
        let result = find_minisig_asset(&assets, "aegis");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "aegis.minisig");
    }

    #[test]
    fn test_is_retryable_status() {
        assert!(is_retryable_status(reqwest::StatusCode::GATEWAY_TIMEOUT)); // 504
        assert!(is_retryable_status(reqwest::StatusCode::BAD_GATEWAY)); // 502
        assert!(is_retryable_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        )); // 503
        assert!(is_retryable_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        )); // 500
        assert!(is_retryable_status(reqwest::StatusCode::REQUEST_TIMEOUT)); // 408
        assert!(is_retryable_status(reqwest::StatusCode::TOO_MANY_REQUESTS)); // 429

        assert!(!is_retryable_status(reqwest::StatusCode::OK));
        assert!(!is_retryable_status(reqwest::StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(reqwest::StatusCode::FORBIDDEN));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
    }

    #[test]
    fn test_is_retryable_anyhow_error_with_status_message() {
        let err504 = anyhow!("HTTP status server error (504 Gateway Timeout) for url");
        assert!(is_retryable_anyhow_error(&err504));

        let err502 = anyhow!("HTTP status server error (502 Bad Gateway) for url");
        assert!(is_retryable_anyhow_error(&err502));

        let err_timeout = anyhow!("Connection timed out");
        assert!(is_retryable_anyhow_error(&err_timeout));

        let err_reset = anyhow!("Connection reset by peer");
        assert!(is_retryable_anyhow_error(&err_reset));

        // 端口号和哈希不应被误判为 504/502/503
        let err_port = anyhow!("Failed to add port 35048 to firewalld");
        assert!(!is_retryable_anyhow_error(&err_port));

        let err_commit = anyhow!("Commit sha a504b12c failed verification");
        assert!(!is_retryable_anyhow_error(&err_commit));

        let err_404 = anyhow!("HTTP status client error (404 Not Found) for url");
        assert!(!is_retryable_anyhow_error(&err_404));

        let err_validation = anyhow!("SHA256 校验失败");
        assert!(!is_retryable_anyhow_error(&err_validation));
    }

    #[tokio::test]
    async fn test_retry_with_backoff_succeeds_on_first_try() {
        let mut calls = 0;
        let res = retry_with_backoff("test_op", 3, Duration::from_millis(1), |_| {
            calls += 1;
            async { Ok::<_, anyhow::Error>(42) }
        })
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_retry_with_backoff_retries_transient_error_and_succeeds() {
        let mut calls = 0;
        let res = retry_with_backoff("test_op", 3, Duration::from_millis(1), |attempt| {
            calls += 1;
            async move {
                if attempt < 3 {
                    anyhow::bail!("504 Gateway Timeout");
                }
                Ok::<_, anyhow::Error>("success")
            }
        })
        .await;
        assert_eq!(res.unwrap(), "success");
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn test_retry_with_backoff_fails_immediately_on_non_retryable_error() {
        let mut calls = 0;
        let res: Result<()> = retry_with_backoff("test_op", 3, Duration::from_millis(1), |_| {
            calls += 1;
            async { anyhow::bail!("404 Not Found") }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn test_find_minisig_asset_not_found() {
        let assets = vec![ReleaseAsset {
            name: "aegis".to_string(),
            browser_download_url: "https://example.com/aegis".to_string(),
            url: String::new(),
            size: None,
            digest: None,
        }];
        assert!(find_minisig_asset(&assets, "aegis").is_none());
    }
}
