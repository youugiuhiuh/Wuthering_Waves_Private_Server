use anyhow::{Context, Result, anyhow};
use reqwest::Url;
use reqwest::header::{ACCEPT, HeaderValue, USER_AGENT};
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

const GITHUB_API_ORIGIN: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlStage {
    Initial,
    Redirect,
}

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
        &self.browser_download_url
    }
}

pub fn validate_asset_url(input: &str, stage: UrlStage) -> Result<Url> {
    let url = Url::parse(input).map_err(|_| anyhow!("Invalid GitHub asset URL"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("GitHub asset URL must use credential-less HTTPS"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("GitHub asset URL missing host"))?;
    let allowed = match stage {
        UrlStage::Initial => host == "github.com",
        UrlStage::Redirect => matches!(
            host,
            "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
        ),
    };
    if !allowed {
        return Err(anyhow!("GitHub asset URL host is not trusted"));
    }
    Ok(url)
}

pub fn github_asset_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many GitHub asset redirects");
            }
            match validate_asset_url(attempt.url().as_str(), UrlStage::Redirect) {
                Ok(_) => attempt.follow(),
                Err(_) => attempt.error("untrusted GitHub asset redirect"),
            }
        }))
        .build()
        .context("Failed to build GitHub asset client")
}

pub fn github_api_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .build()
        .context("Failed to build GitHub API client")
}

pub fn build_asset_request(
    client: &reqwest::Client,
    input: &str,
) -> Result<reqwest::RequestBuilder> {
    let url = validate_asset_url(input, UrlStage::Initial)?;
    Ok(client
        .get(url)
        .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE)))
}

pub fn build_github_api_request(
    client: &reqwest::Client,
    api_path: &str,
    token: Option<&str>,
) -> Result<reqwest::RequestBuilder> {
    if api_path.contains('?')
        || api_path.contains('#')
        || api_path.contains("..")
        || api_path.contains("//")
        || api_path.contains('@')
    {
        anyhow::bail!("GitHub API 路径包含不安全的字符");
    }
    let url = Url::parse(&format!(
        "{}/{}",
        GITHUB_API_ORIGIN,
        api_path.trim_start_matches('/')
    ))
    .context("Failed to build GitHub API URL")?;
    let mut request = client
        .get(url)
        .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
        .header(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request)
}

pub fn build_github_api_query_request(
    client: &reqwest::Client,
    api_path: &str,
    query: &[(&str, &str)],
    token: Option<&str>,
) -> Result<reqwest::RequestBuilder> {
    Ok(build_github_api_request(client, api_path, token)?.query(query))
}

async fn send_github_json<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    request
        .send()
        .await
        .context("GitHub API request failed")?
        .error_for_status()
        .context("GitHub API returned error status")?
        .json::<T>()
        .await
        .context("Failed to parse GitHub API response")
}

pub async fn fetch_github_json_with_query<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_path: &str,
    query: &[(&str, &str)],
    token: Option<&str>,
) -> Result<T> {
    send_github_json(build_github_api_query_request(
        client, api_path, query, token,
    )?)
    .await
}

pub async fn fetch_github_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_path: &str,
    token: Option<&str>,
) -> Result<T> {
    send_github_json(build_github_api_request(client, api_path, token)?).await
}

fn normalized_sha256(value: &str) -> Option<String> {
    let value = value.trim();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

pub fn parse_digest(input: &str) -> Option<String> {
    normalized_sha256(input.strip_prefix("sha256:")?)
}

pub fn parse_xray_sha256_dgst(input: &str) -> Result<String> {
    let dgst = parse_xray_dgst(input)?;
    Ok(dgst.sha2_256)
}

#[allow(dead_code)]
struct XrayDgst {
    md5: String,
    sha1: String,
    sha2_256: String,
    sha2_512: String,
}

fn parse_xray_dgst(input: &str) -> Result<XrayDgst> {
    let mut md5 = None;
    let mut sha1 = None;
    let mut sha2_256 = None;
    let mut sha2_512 = None;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            anyhow::bail!(".dgst 行缺少 '=': {line}");
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "MD5" => {
                if md5.is_some() {
                    anyhow::bail!(".dgst 重复 MD5");
                }
                if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    anyhow::bail!(".dgst MD5 格式无效");
                }
                md5 = Some(value.to_ascii_lowercase());
            }
            "SHA1" => {
                if sha1.is_some() {
                    anyhow::bail!(".dgst 重复 SHA1");
                }
                if value.len() != 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    anyhow::bail!(".dgst SHA1 格式无效");
                }
                sha1 = Some(value.to_ascii_lowercase());
            }
            "SHA2-256" => {
                if sha2_256.is_some() {
                    anyhow::bail!(".dgst 重复 SHA2-256");
                }
                sha2_256 = Some(
                    normalized_sha256(value).ok_or_else(|| anyhow!(".dgst SHA2-256 格式无效"))?,
                );
            }
            "SHA2-512" => {
                if sha2_512.is_some() {
                    anyhow::bail!(".dgst 重复 SHA2-512");
                }
                if value.len() != 128 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    anyhow::bail!(".dgst SHA2-512 格式无效");
                }
                sha2_512 = Some(value.to_ascii_lowercase());
            }
            _ => anyhow::bail!(".dgst 包含未知算法: {key}"),
        }
    }

    Ok(XrayDgst {
        md5: md5.ok_or_else(|| anyhow!(".dgst 缺少 MD5"))?,
        sha1: sha1.ok_or_else(|| anyhow!(".dgst 缺少 SHA1"))?,
        sha2_256: sha2_256.ok_or_else(|| anyhow!(".dgst 缺少 SHA2-256"))?,
        sha2_512: sha2_512.ok_or_else(|| anyhow!(".dgst 缺少 SHA2-512"))?,
    })
}

pub fn find_named_asset<'a>(assets: &'a [ReleaseAsset], name: &str) -> Option<&'a ReleaseAsset> {
    assets.iter().find(|asset| asset.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
    fn browser_download_url_is_required() {
        let asset = ReleaseAsset {
            name: "aegis".into(),
            browser_download_url: String::new(),
            url: "https://api.github.com/repos/o/r/releases/assets/1".into(),
            size: None,
            digest: None,
        };
        assert_eq!(asset.download_url(), "");
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
    fn initial_asset_url_requires_exact_github_https_origin() {
        assert!(
            validate_asset_url(
                "https://github.com/o/r/releases/download/v1/aegis",
                UrlStage::Initial
            )
            .is_ok()
        );
        for url in [
            "http://github.com/o/r/releases/download/v1/aegis",
            "https://github.com.evil.test/aegis",
            "https://127.0.0.1/aegis",
            "https://user@github.com/aegis",
        ] {
            assert!(
                validate_asset_url(url, UrlStage::Initial).is_err(),
                "accepted {url}"
            );
        }
    }

    #[test]
    fn redirects_require_exact_github_asset_hosts() {
        for url in [
            "https://release-assets.githubusercontent.com/object",
            "https://objects.githubusercontent.com/object",
        ] {
            assert!(validate_asset_url(url, UrlStage::Redirect).is_ok());
        }
        assert!(
            validate_asset_url(
                "https://release-assets.githubusercontent.com.evil.test/object",
                UrlStage::Redirect
            )
            .is_err()
        );
        assert!(
            validate_asset_url(
                "https://github.com/o/r/releases/download/v1/aegis",
                UrlStage::Redirect
            )
            .is_err()
        );
    }

    #[test]
    fn asset_request_never_contains_authorization() {
        let client = github_asset_client(Duration::from_secs(1)).unwrap();
        let request =
            build_asset_request(&client, "https://github.com/o/r/releases/download/v1/aegis")
                .unwrap()
                .build()
                .unwrap();
        assert!(
            !request
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION)
        );
    }

    #[test]
    fn api_request_is_fixed_origin_and_may_contain_authorization() {
        let client = github_api_client(Duration::from_secs(1)).unwrap();
        let request =
            build_github_api_request(&client, "repos/o/r/releases/latest", Some("secret"))
                .unwrap()
                .build()
                .unwrap();
        assert_eq!(
            request.url().origin().ascii_serialization(),
            "https://api.github.com"
        );
        assert!(
            request
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION)
        );
    }

    #[test]
    fn parses_exactly_one_xray_sha2_256() {
        let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
        let valid = format!(
            "MD5= ee4e2ff74948a9b464624b1cabc44409\n\
             SHA1= b55b06e74e89083b9cedfdecf0d68b579cd2af72\n\
             SHA2-256= {hash}\n\
             SHA2-512= e8bc40a0687cac184bbe4b5c1f047e69064ccedc489fb25e208889ae287bbf8736dff16b108d68fc00dc33edc8bb53502e47a9698a277f4f51b67b83d899e518\n"
        );
        assert_eq!(parse_xray_sha256_dgst(&valid).unwrap(), hash);
    }

    #[test]
    fn rejects_xray_dgst_missing_fields() {
        let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
        assert!(parse_xray_sha256_dgst(&format!("SHA2-256= {hash}\n")).is_err());
        assert!(
            parse_xray_sha256_dgst("SHA1= b55b06e74e89083b9cedfdecf0d68b579cd2af72\n").is_err()
        );
    }

    #[test]
    fn rejects_xray_dgst_duplicate_sha2_256() {
        let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
        let dup = format!(
            "MD5= ee4e2ff74948a9b464624b1cabc44409\n\
             SHA1= b55b06e74e89083b9cedfdecf0d68b579cd2af72\n\
             SHA2-256= {hash}\n\
             SHA2-256= {hash}\n\
             SHA2-512= e8bc40a0687cac184bbe4b5c1f047e69064ccedc489fb25e208889ae287bbf8736dff16b108d68fc00dc33edc8bb53502e47a9698a277f4f51b67b83d899e518\n"
        );
        assert!(parse_xray_sha256_dgst(&dup).is_err());
    }

    #[test]
    fn rejects_xray_dgst_wrong_algorithm_name() {
        let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
        let wrong = format!(
            "MD5= ee4e2ff74948a9b464624b1cabc44409\n\
             SHA1= b55b06e74e89083b9cedfdecf0d68b579cd2af72\n\
             SHA256= {hash}\n\
             SHA2-512= e8bc40a0687cac184bbe4b5c1f047e69064ccedc489fb25e208889ae287bbf8736dff16b108d68fc00dc33edc8bb53502e47a9698a277f4f51b67b83d899e518\n"
        );
        assert!(parse_xray_sha256_dgst(&wrong).is_err());
    }

    #[test]
    fn github_api_query_is_structured_and_percent_encoded() {
        let client = github_api_client(Duration::from_secs(1)).unwrap();
        let request = build_github_api_query_request(
            &client,
            "repos/XTLS/Xray-core/releases",
            &[("per_page", "5&unexpected=true")],
            Some("secret"),
        )
        .unwrap()
        .build()
        .unwrap();

        assert_eq!(
            request.url().as_str(),
            "https://api.github.com/repos/XTLS/Xray-core/releases?per_page=5%26unexpected%3Dtrue"
        );
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .unwrap(),
            "Bearer secret"
        );
    }

    #[test]
    fn github_api_query_does_not_relax_path_validation() {
        let client = github_api_client(Duration::from_secs(1)).unwrap();
        assert!(
            build_github_api_query_request(
                &client,
                "repos/XTLS/Xray-core/releases?per_page=5",
                &[],
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_xray_dgst_invalid_md5_length() {
        let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
        let bad_md5 = format!(
            "MD5= xyz\n\
             SHA1= b55b06e74e89083b9cedfdecf0d68b579cd2af72\n\
             SHA2-256= {hash}\n\
             SHA2-512= e8bc40a0687cac184bbe4b5c1f047e69064ccedc489fb25e208889ae287bbf8736dff16b108d68fc00dc33edc8bb53502e47a9698a277f4f51b67b83d899e518\n"
        );
        assert!(parse_xray_sha256_dgst(&bad_md5).is_err());
    }
}
