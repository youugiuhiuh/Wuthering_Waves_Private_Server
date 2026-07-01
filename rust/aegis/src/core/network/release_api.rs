use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;

pub static SHA256_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)sha256[:\s]+([0-9a-f]{64})").expect("valid sha256 regex"));

const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

#[derive(Debug, Deserialize)]
pub struct ReleaseResponse {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
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

pub fn parse_digest(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    lower
        .strip_prefix("sha256:")
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() == 64)
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
