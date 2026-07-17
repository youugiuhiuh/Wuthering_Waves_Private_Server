# Aegis GitHub-only Update Security Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restrict both updater chains to fixed GitHub repositories, prevent credential-bearing asset requests, require Aegis Minisign verification, and require Xray API/dgst/download SHA256 agreement before installation.

**Architecture:** Keep the existing updater modules and put the small shared trust primitives in `core/network/release_api.rs`. Release metadata uses a fixed-origin API helper that may carry `GITHUB_TOKEN`; asset clients never receive a token and validate the initial URL plus every redirect. Aegis and Xray retain separate verification policies because only Aegis has an independent Minisign signature.

**Tech Stack:** Rust 2024, Tokio, reqwest, serde, sha2, minisign-verify, anyhow, existing unit-test framework

## Global Constraints

- Aegis repository is exactly `youugiuhiuh/Wuthering_Waves_Private_Server`; its asset is exactly `aegis`.
- Xray repository is exactly `XTLS/Xray-core`; architecture selects `Xray-linux-64.zip` or `Xray-linux-arm64-v8a.zip`.
- Release metadata origin is exactly `https://api.github.com`.
- `GITHUB_TOKEN` may be attached only to requests whose origin is exactly `https://api.github.com`.
- Initial asset URLs must be HTTPS on `github.com`; redirects may use only `release-assets.githubusercontent.com` or `objects.githubusercontent.com`.
- Reject HTTP, IP literals, userinfo, wildcard/suffix host matches, and every unlisted host.
- Use only `browser_download_url`; never fall back to the GitHub API asset `url`.
- Aegis requires an API SHA256 digest, an unexpired pinned Minisign key, and exact trusted-comment tag and asset matches.
- Xray requires one strict `.dgst` `SHA2-256` value and equality among API digest, `.dgst`, and downloaded bytes.
- Do not add a compatibility switch, bypass, mirror, generic trust-policy abstraction, or new dependency.
- Validation must finish before replacement, extraction, or service restart; validation failures remove temporary downloads.
- Do not address rollback, single-flight, process lifecycle, or unrelated audit findings in this change.
- Do not commit unless the user explicitly requests it; commit commands below are execution checkpoints, not authorization to commit.

## File Map

- Modify `rust/aegis/src/core/network/release_api.rs`: fixed GitHub API request helper, token-free asset client, URL/redirect validators, strict digest parsers.
- Modify `rust/aegis/src/core/system/upgrade.rs`: fixed Aegis source and mandatory Minisign path.
- Modify `rust/aegis/src/core/crypto/minisign.rs`: exact trusted-comment parsing used by Aegis verification.
- Modify `rust/aegis/src/core/system/core_upgrade.rs`: fixed Xray source and three-way digest policy.
- Modify `rust/aegis/src/core/xray/installer.rs`: adapt the Xray config constructor after owner/repo removal.
- Modify `docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`: mark `AEGIS-002` and `AEGIS-003` complete only after all verification passes.

## Baseline Record

- [ ] Run `cargo fmt --check` from `rust/aegis` and save the exact pre-existing output in the execution notes.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings` from `rust/aegis` and record the current 14-failure baseline by file and lint.
- [ ] Run `cargo test --all-features` from `rust/aegis` and record pass/fail counts before edits.
- [ ] Do not suppress, allow, or mix unrelated baseline cleanup into the security patch; if a baseline error touches an edited line, fix only that local error and identify it in review.

---

### Task 1: Shared GitHub Request Boundary

**Files:**
- Modify: `rust/aegis/src/core/network/release_api.rs`

**Interfaces:**
- Produces: `fetch_github_json<T>(client: &reqwest::Client, api_path: &str, token: Option<&str>) -> Result<T>`
- Produces: `github_api_client(timeout: Duration) -> Result<reqwest::Client>` and `build_github_api_request(...) -> Result<reqwest::RequestBuilder>`
- Produces: `github_asset_client(timeout: Duration) -> Result<reqwest::Client>`
- Produces: `build_asset_request(client: &reqwest::Client, url: &str) -> Result<reqwest::RequestBuilder>`
- Produces: `parse_digest(input: &str) -> Option<String>` with strict lowercase-normalized SHA256 validation
- Produces: `parse_xray_sha256_dgst(input: &str) -> Result<String>`
- Produces: `find_named_asset<'a>(assets: &'a [ReleaseAsset], name: &str) -> Option<&'a ReleaseAsset>`
- Consumes: existing `ReleaseResponse` and `ReleaseAsset` serde models

- [ ] **Step 1: Replace fallback and mirror tests with failing trust-boundary tests**

Replace the download URL fallback test and add the following tests inside `release_api.rs`:

```rust
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
fn initial_asset_url_requires_exact_github_https_origin() {
    assert!(validate_asset_url("https://github.com/o/r/releases/download/v1/aegis", UrlStage::Initial).is_ok());
    for url in [
        "http://github.com/o/r/releases/download/v1/aegis",
        "https://github.com.evil.test/aegis",
        "https://127.0.0.1/aegis",
        "https://user@github.com/aegis",
    ] {
        assert!(validate_asset_url(url, UrlStage::Initial).is_err(), "accepted {url}");
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
    assert!(validate_asset_url("https://release-assets.githubusercontent.com.evil.test/object", UrlStage::Redirect).is_err());
    assert!(validate_asset_url("https://github.com/o/r/releases/download/v1/aegis", UrlStage::Redirect).is_err());
}

#[test]
fn asset_request_never_contains_authorization() {
    let client = github_asset_client(Duration::from_secs(1)).unwrap();
    let request = build_asset_request(
        &client,
        "https://github.com/o/r/releases/download/v1/aegis",
    )
    .unwrap()
    .build()
    .unwrap();
    assert!(!request.headers().contains_key(reqwest::header::AUTHORIZATION));
}

#[test]
fn api_request_is_fixed_origin_and_may_contain_authorization() {
    let client = github_api_client(Duration::from_secs(1)).unwrap();
    let request = build_github_api_request(&client, "repos/o/r/releases/latest", Some("secret"))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.url().origin().ascii_serialization(), "https://api.github.com");
    assert!(request.headers().contains_key(reqwest::header::AUTHORIZATION));
}

#[test]
fn parses_exactly_one_xray_sha2_256() {
    let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
    let valid = format!("MD5= deadbeef\nSHA2-256= {hash}\nSHA2-512= deadbeef\n");
    assert_eq!(parse_xray_sha256_dgst(&valid).unwrap(), hash);
    assert!(parse_xray_sha256_dgst(&format!("SHA2-256= {hash}\nSHA2-256= {hash}\n")).is_err());
    assert!(parse_xray_sha256_dgst("SHA2-256= xyz").is_err());
    assert!(parse_xray_sha256_dgst("SHA256= 23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae").is_err());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test core::network::release_api::tests --all-features`

Expected: compilation fails because the GitHub client/request helpers, `UrlStage`, `validate_asset_url`, and `parse_xray_sha256_dgst` do not exist; the fallback URL assertion also fails.

- [ ] **Step 3: Implement fixed-origin API and token-free asset helpers**

Add the fixed GitHub helpers and tighten `ReleaseAsset::download_url`. Keep the old mirror function temporarily so the unmodified updater callers compile; Task 3 deletes it after the final caller migrates.

```rust
use anyhow::{Context, Result, anyhow};
use reqwest::redirect::Policy;
use reqwest::Url;
use std::time::Duration;

const GITHUB_API_ORIGIN: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlStage {
    Initial,
    Redirect,
}

impl ReleaseAsset {
    pub fn download_url(&self) -> &str {
        &self.browser_download_url
    }
}

pub fn validate_asset_url(input: &str, stage: UrlStage) -> Result<Url> {
    let url = Url::parse(input).map_err(|_| anyhow!("无效的 GitHub 资产 URL"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("GitHub 资产 URL 必须使用无凭据 HTTPS"));
    }
    let host = url.host_str().ok_or_else(|| anyhow!("GitHub 资产 URL 缺少 host"))?;
    let allowed = match stage {
        UrlStage::Initial => host == "github.com",
        UrlStage::Redirect => matches!(
            host,
            "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
        ),
    };
    if !allowed {
        return Err(anyhow!("GitHub 资产 URL host 不受信任"));
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
        .context("构建 GitHub 资产客户端失败")
}

pub fn github_api_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .build()
        .context("构建 GitHub API 客户端失败")
}

pub fn build_asset_request(client: &reqwest::Client, input: &str) -> Result<reqwest::RequestBuilder> {
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
    let url = Url::parse(&format!(
        "{}/{}",
        GITHUB_API_ORIGIN,
        api_path.trim_start_matches('/')
    ))
    .context("构建 GitHub API URL 失败")?;
    let mut request = client
        .get(url)
        .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
        .header(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request)
}

pub async fn fetch_github_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_path: &str,
    token: Option<&str>,
) -> Result<T> {
    build_github_api_request(client, api_path, token)?
        .send()
        .await
        .context("GitHub API 请求失败")?
        .error_for_status()
        .context("GitHub API 返回错误状态")?
        .json::<T>()
        .await
        .context("解析 GitHub API 响应失败")
}
```

Use `reqwest::Url` instead of adding a direct `url` dependency if `url` is not already a direct dependency. Keep `ReleaseAsset.url` only as a deserialized but unused compatibility field until the whole crate compiles; it must never be read.

- [ ] **Step 4: Implement strict digest parsers and named-asset lookup**

```rust
fn normalized_sha256(value: &str) -> Option<String> {
    let value = value.trim();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

pub fn parse_digest(input: &str) -> Option<String> {
    normalized_sha256(input.strip_prefix("sha256:")?)
}

pub fn parse_xray_sha256_dgst(input: &str) -> Result<String> {
    let values: Vec<String> = input
        .lines()
        .filter_map(|line| line.trim().strip_prefix("SHA2-256= "))
        .map(|value| normalized_sha256(value).ok_or_else(|| anyhow!("无效的 Xray SHA2-256")))
        .collect::<Result<_>>()?;
    match values.as_slice() {
        [value] => Ok(value.clone()),
        [] => Err(anyhow!("Xray .dgst 缺少 SHA2-256")),
        _ => Err(anyhow!("Xray .dgst 包含重复 SHA2-256")),
    }
}

pub fn find_named_asset<'a>(assets: &'a [ReleaseAsset], name: &str) -> Option<&'a ReleaseAsset> {
    assets.iter().find(|asset| asset.name == name)
}
```

Keep `SHA256_LINE_RE`, `parse_sha256_manifest`, `extract_sha256_from_body`, and mirror retry code only until Tasks 2 and 3 have removed their callers; delete all of them in Task 3.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run: `cargo test core::network::release_api::tests --all-features`

Expected: all release API tests pass; no test expects fallback to `ReleaseAsset.url`.

- [ ] **Step 6: Review checkpoint**

Inspect: `git diff -- rust/aegis/src/core/network/release_api.rs`

Expected: one fixed API origin, exact host equality, a token-free asset request API, no wildcard matching, and no mirror loop.

- [ ] **Step 7: Commit checkpoint if explicitly authorized**

```bash
git add rust/aegis/src/core/network/release_api.rs
git commit -m "security: constrain updater network requests"
```

---

### Task 2: Fixed Aegis Source and Mandatory Minisign

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Modify: `rust/aegis/src/core/crypto/minisign.rs`

**Interfaces:**
- Consumes: `fetch_github_json`, `github_asset_client`, `build_asset_request`, `parse_digest`, and `find_named_asset` from Task 1
- Produces: `validate_trusted_comment(comment: &str, expected_tag: &str, expected_asset: &str) -> Result<()>`
- Produces: `verify_downloaded_update(path: &Path, artifact: &ReleaseArtifact) -> Result<()>`, which removes the temporary update on verification failure
- Produces: `ReleaseArtifact { tag_name, asset_name, download_url, sha256, size, minisig }` where `minisig` is required `Vec<u8>`

- [ ] **Step 1: Write failing tests for fixed identity and exact signature metadata**

Replace `test_parse_release_repo` and add:

```rust
#[test]
fn aegis_release_identity_is_fixed() {
    assert_eq!(AEGIS_RELEASE_OWNER, "youugiuhiuh");
    assert_eq!(AEGIS_RELEASE_REPO, "Wuthering_Waves_Private_Server");
    assert_eq!(AEGIS_RELEASE_ASSET, "aegis");
    assert_eq!(aegis_release_path(), "repos/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest");
}

#[test]
fn trusted_comment_requires_exact_tag_and_asset() {
    validate_trusted_comment("v3.4.4:aegis", "v3.4.4", "aegis").unwrap();
    assert!(validate_trusted_comment("release-v3.4.4:aegis", "v3.4.4", "aegis").is_err());
    assert!(validate_trusted_comment("v3.4.4:aegis:extra", "v3.4.4", "aegis").is_err());
    assert!(validate_trusted_comment("v3.4.4:other", "v3.4.4", "aegis").is_err());
}

#[test]
fn release_artifact_requires_signature_bytes() {
    let artifact = ReleaseArtifact {
        repository: "youugiuhiuh/Wuthering_Waves_Private_Server".into(),
        tag_name: "v3.4.4".into(),
        asset_name: "aegis".into(),
        download_url: "https://github.com/o/r/releases/download/v3.4.4/aegis".into(),
        sha256: "0".repeat(64),
        size: None,
        minisig: vec![1],
    };
    assert_eq!(artifact.minisig, vec![1]);
}

#[tokio::test]
async fn failed_signature_verification_removes_temporary_update() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("aegis.update");
    tokio::fs::write(&path, b"downloaded binary").await.unwrap();
    let artifact = ReleaseArtifact {
        repository: "youugiuhiuh/Wuthering_Waves_Private_Server".into(),
        tag_name: "v3.4.4".into(),
        asset_name: "aegis".into(),
        download_url: "https://github.com/o/r/releases/download/v3.4.4/aegis".into(),
        sha256: hex::encode(Sha256::digest(b"downloaded binary")),
        size: None,
        minisig: b"invalid signature".to_vec(),
    };
    let manager = UpgradeManager::new().unwrap();
    assert!(manager.verify_downloaded_update(&path, &artifact).await.is_err());
    assert!(!path.exists());
}
```

In `minisign.rs`, replace the multi-colon acceptance test with:

```rust
#[test]
fn test_parse_trusted_comment_rejects_extra_fields() {
    assert!(parse_trusted_comment("v1.0.0:file:extra").is_err());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test core::system::upgrade::tests --all-features
cargo test core::crypto::minisign::tests --all-features
```

Expected: tests fail because fixed constants/path and `validate_trusted_comment` do not exist, `minisig` is optional, and multi-colon comments are accepted.

- [ ] **Step 3: Remove all configurable Aegis release sources**

Replace repository/configuration parsing and manager fields with:

```rust
const AEGIS_RELEASE_OWNER: &str = "youugiuhiuh";
const AEGIS_RELEASE_REPO: &str = "Wuthering_Waves_Private_Server";
const AEGIS_RELEASE_ASSET: &str = "aegis";
const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

fn aegis_release_path() -> String {
    format!(
        "repos/{AEGIS_RELEASE_OWNER}/{AEGIS_RELEASE_REPO}/releases/latest"
    )
}

pub struct UpgradeManager {
    api_client: reqwest::Client,
    asset_client: reqwest::Client,
    token: Option<String>,
}

impl UpgradeManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            api_client: github_api_client(Duration::from_secs(60))?,
            asset_client: github_asset_client(Duration::from_secs(60))?,
            token: env::var("GITHUB_TOKEN").ok().filter(|value| !value.is_empty()),
        })
    }
}
```

Delete every read of `AEGIS_RELEASE_MIRRORS`, `AEGIS_RELEASE_REPOSITORIES`, `AEGIS_RELEASE_REPOSITORY`, `AEGIS_RELEASE_OWNER`, `AEGIS_RELEASE_REPO`, and `AEGIS_RELEASE_ASSET`.

- [ ] **Step 4: Make release selection, API digest, and Minisign fail closed**

Change `ReleaseArtifact.minisig` to `Vec<u8>`. Replace repository fallback, fuzzy asset selection, SHA fallbacks, and optional signature download with exact lookup:

```rust
async fn fetch_latest_release(&self) -> Result<ReleaseArtifact> {
    let release: ReleaseResponse = fetch_github_json(
        &self.api_client,
        &aegis_release_path(),
        self.token.as_deref(),
    )
    .await?;
    if release.tag_name.is_empty() {
        anyhow::bail!("Release 缺少 tag_name");
    }

    let asset = find_named_asset(&release.assets, AEGIS_RELEASE_ASSET)
        .ok_or_else(|| anyhow!("Release 缺少固定 Aegis 资产"))?;
    let signature = find_named_asset(&release.assets, &format!("{AEGIS_RELEASE_ASSET}.minisig"))
        .ok_or_else(|| anyhow!("Release 缺少 Aegis Minisign 签名"))?;
    let download_url = asset.download_url();
    if download_url.is_empty() {
        anyhow::bail!("Aegis 资产缺少 browser_download_url");
    }
    let signature_url = signature.download_url();
    if signature_url.is_empty() {
        anyhow::bail!("Aegis 签名缺少 browser_download_url");
    }
    let sha256 = parse_digest(
        asset.digest.as_deref().ok_or_else(|| anyhow!("Aegis 资产缺少 API digest"))?,
    )
    .ok_or_else(|| anyhow!("Aegis API digest 格式无效"))?;
    let minisig = build_asset_request(&self.asset_client, signature_url)?
        .send()
        .await
        .context("下载 Aegis Minisign 失败")?
        .error_for_status()
        .context("Aegis Minisign 返回错误状态")?
        .bytes()
        .await
        .context("读取 Aegis Minisign 失败")?
        .to_vec();

    Ok(ReleaseArtifact {
        repository: format!("{AEGIS_RELEASE_OWNER}/{AEGIS_RELEASE_REPO}"),
        tag_name: release.tag_name,
        asset_name: AEGIS_RELEASE_ASSET.into(),
        download_url: download_url.into(),
        sha256,
        size: asset.size,
        minisig,
    })
}
```

Use `build_asset_request(&self.asset_client, &artifact.download_url)?` for the binary download. Delete `select_asset`, `download_sha256_manifest`, `download_minisig`, and the old credential-bearing `build_request`.

- [ ] **Step 5: Enforce exact trusted-comment parsing and unconditional verification**

In `minisign.rs`:

```rust
pub fn parse_trusted_comment(comment: &str) -> Result<(String, String)> {
    let mut parts = comment.split(':');
    let version = parts.next().unwrap_or_default();
    let asset = parts.next().unwrap_or_default();
    if version.is_empty() || asset.is_empty() || parts.next().is_some() {
        return Err(anyhow!("无效的可信注释格式"));
    }
    Ok((version.to_string(), asset.to_string()))
}
```

In `upgrade.rs`:

```rust
fn validate_trusted_comment(comment: &str, expected_tag: &str, expected_asset: &str) -> Result<()> {
    let (tag, asset) = minisign::parse_trusted_comment(comment)?;
    if tag != expected_tag || asset != expected_asset {
        anyhow::bail!("Minisign trusted comment 与 Release 不匹配");
    }
    Ok(())
}
```

After streaming finishes, route both checks through this helper before returning the path:

```rust
async fn verify_downloaded_update(&self, path: &Path, artifact: &ReleaseArtifact) -> Result<()> {
    let result = async {
        let data = fs::read(path).await.context("读取 Aegis 更新文件失败")?;
        let actual_sha256 = hex::encode(Sha256::digest(&data));
        if actual_sha256 != artifact.sha256 {
            anyhow::bail!("Aegis SHA256 校验失败");
        }
        let signature = std::str::from_utf8(&artifact.minisig)
            .context("Aegis Minisign 不是有效 UTF-8")?;
        let info = minisign::verify_minisign(&data, signature, MINISIGN_PUBLIC_KEYS)?;
        validate_trusted_comment(
            &info.trusted_comment,
            &artifact.tag_name,
            &artifact.asset_name,
        )
    }
    .await;
    if result.is_err() {
        fs::remove_file(path).await.ok();
    }
    result
}
```

Call `verify_downloaded_update` at the end of `download_with_progress` and return the update path only after it succeeds. Remove the old streaming `hasher` variable and updates, inline hash block, `verify_downloaded_minisign`, and the `if let Some(sig_bytes)` branch. `run` already calls `finalize_install` only after `download_with_progress` returns `Ok`, so this preserves the pre-install security boundary.

- [ ] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
cargo test core::system::upgrade::tests --all-features
cargo test core::crypto::minisign::tests --all-features
```

Expected: all Aegis updater and Minisign tests pass.

- [ ] **Step 7: Verify removed Aegis configuration surface**

Run: `rg '"(AEGIS_RELEASE_MIRRORS|AEGIS_RELEASE_REPOSITORIES|AEGIS_RELEASE_REPOSITORY|AEGIS_RELEASE_OWNER|AEGIS_RELEASE_REPO|AEGIS_RELEASE_ASSET|NicholasDewar)|https://(codeberg|gitea)' src/core/system/upgrade.rs src/core/network/release_api.rs`

Expected: no matches.

- [ ] **Step 8: Commit checkpoint if explicitly authorized**

```bash
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/crypto/minisign.rs
git commit -m "security: require signed Aegis updates"
```

---

### Task 3: Fixed Xray Source and Three-way Digest Verification

**Files:**
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`
- Modify: `rust/aegis/src/core/xray/installer.rs`

**Interfaces:**
- Consumes: Task 1 GitHub API, asset client, exact asset lookup, API digest parser, and `.dgst` parser
- Produces: `WwpsCoreReleaseInfo { tag_name, asset_name, download_url, api_sha256, dgst_sha256, size }`
- Produces: `verify_xray_hashes(actual: &str, api: &str, dgst: &str) -> Result<()>`
- Produces: `verify_xray_archive(path: &Path, actual: &str, release: &WwpsCoreReleaseInfo) -> Result<()>`, which removes a mismatched archive
- Produces: `WwpsCoreUpgradeConfig::new(service_name, install_dir, backup_dir, temp_dir, arch)` with no remote owner/repo parameters

- [ ] **Step 1: Write failing fixed-source and digest tests**

Replace mirror override tests and update config construction tests:

```rust
#[test]
fn xray_release_identity_is_fixed() {
    assert_eq!(XRAY_RELEASE_OWNER, "XTLS");
    assert_eq!(XRAY_RELEASE_REPO, "Xray-core");
    assert_eq!(xray_release_path(None), "repos/XTLS/Xray-core/releases/latest");
    assert_eq!(
        xray_release_path(Some("v26.3.27")),
        "repos/XTLS/Xray-core/releases/tags/v26.3.27"
    );
}

#[test]
fn xray_hashes_require_three_way_equality() {
    let hash = "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae";
    verify_xray_hashes(hash, hash, hash).unwrap();
    assert!(verify_xray_hashes(&"0".repeat(64), hash, hash).is_err());
    assert!(verify_xray_hashes(hash, &"0".repeat(64), hash).is_err());
    assert!(verify_xray_hashes(hash, hash, &"0".repeat(64)).is_err());
}

#[test]
fn config_has_no_remote_repository_fields() {
    let tmp = tempdir().unwrap();
    let config = WwpsCoreUpgradeConfig::new(
        "wwps-core",
        tmp.path().join("install"),
        tmp.path().join("backup"),
        tmp.path().join("temp"),
        CpuArch::Amd64,
    );
    assert_eq!(config.service_name, "wwps-core");
}

#[tokio::test]
async fn failed_xray_digest_verification_removes_temporary_archive() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("xray.zip");
    tokio::fs::write(&archive, b"archive").await.unwrap();
    let release = WwpsCoreReleaseInfo {
        tag_name: "v26.3.27".into(),
        asset_name: "Xray-linux-64.zip".into(),
        download_url: "https://github.com/XTLS/Xray-core/releases/download/v26.3.27/Xray-linux-64.zip".into(),
        api_sha256: "1".repeat(64),
        dgst_sha256: "1".repeat(64),
        size: None,
    };
    assert!(verify_xray_archive(&archive, &"0".repeat(64), &release).await.is_err());
    assert!(!archive.exists());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test core::system::core_upgrade::tests --all-features`

Expected: compilation fails because fixed path functions and `verify_xray_hashes` do not exist and the config constructor still requires owner/repo.

- [ ] **Step 3: Remove remote identity from Xray runtime configuration**

Use fixed constants and remove `owner`/`repo` fields:

```rust
const XRAY_RELEASE_OWNER: &str = "XTLS";
const XRAY_RELEASE_REPO: &str = "Xray-core";

fn xray_release_path(tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases/tags/{tag}"),
        None => format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases/latest"),
    }
}

pub struct WwpsCoreUpgradeConfig {
    pub service_name: String,
    pub install_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub arch: CpuArch,
}
```

Change `WwpsCoreUpgradeConfig::new` to accept only the five fields above. `from_env()` must keep service/install/backup/temp settings but delete reads of `WWPS_CORE_RELEASE_OWNER` and `WWPS_CORE_RELEASE_REPO`. Update `install_wwps_core` in `core/xray/installer.rs` to remove its `"XTLS", "Xray-core"` constructor arguments.

- [ ] **Step 4: Split API and asset clients and fetch only fixed GitHub metadata**

```rust
pub struct WwpsCoreUpgradeManager {
    config: Arc<WwpsCoreUpgradeConfig>,
    api_client: reqwest::Client,
    asset_client: reqwest::Client,
    github_token: Option<String>,
}

pub fn new(config: WwpsCoreUpgradeConfig) -> Result<Self> {
    Ok(Self {
        config: Arc::new(config),
        api_client: github_api_client(Duration::from_secs(60))?,
        asset_client: github_asset_client(Duration::from_secs(60))?,
        github_token: env::var("GITHUB_TOKEN").ok().filter(|value| !value.is_empty()),
    })
}
```

Use `fetch_github_json(&self.api_client, &xray_release_path(tag), self.github_token.as_deref())` in `fetch_release`. For recent tags, construct only `repos/XTLS/Xray-core/releases?per_page={limit}`.

- [ ] **Step 5: Require exact Xray asset, API digest, and `.dgst`**

Replace `WwpsCoreReleaseInfo` and metadata selection:

```rust
pub struct WwpsCoreReleaseInfo {
    pub tag_name: String,
    pub asset_name: String,
    pub download_url: String,
    pub api_sha256: String,
    pub dgst_sha256: String,
    pub size: Option<u64>,
}

let asset_name = format!("{}.zip", self.config.arch.asset_basename());
let dgst_name = format!("{asset_name}.dgst");
let asset = find_named_asset(&release.assets, &asset_name)
    .ok_or_else(|| anyhow!("Release 缺少固定 Xray 资产"))?;
let dgst_asset = find_named_asset(&release.assets, &dgst_name)
    .ok_or_else(|| anyhow!("Release 缺少 Xray .dgst"))?;
let download_url = asset.download_url();
if download_url.is_empty() {
    anyhow::bail!("Xray 资产缺少 browser_download_url");
}
let dgst_url = dgst_asset.download_url();
if dgst_url.is_empty() {
    anyhow::bail!("Xray .dgst 缺少 browser_download_url");
}
let api_sha256 = parse_digest(
    asset.digest.as_deref().ok_or_else(|| anyhow!("Xray 资产缺少 API digest"))?,
)
.ok_or_else(|| anyhow!("Xray API digest 格式无效"))?;
let dgst_text = build_asset_request(&self.asset_client, dgst_url)?
    .send()
    .await
    .context("下载 Xray .dgst 失败")?
    .error_for_status()
    .context("Xray .dgst 返回错误状态")?
    .text()
    .await
    .context("读取 Xray .dgst 失败")?;
let dgst_sha256 = parse_xray_sha256_dgst(&dgst_text)?;

Ok(WwpsCoreReleaseInfo {
    tag_name: release.tag_name,
    asset_name,
    download_url: download_url.into(),
    api_sha256,
    dgst_sha256,
    size: asset.size,
})
```

Delete all body/manifest fallback and optional Minisign logic from this updater. At this point no caller remains, so also delete `fetch_json_from_mirrors`, `SHA256_LINE_RE`, `parse_sha256_manifest`, and `extract_sha256_from_body` from `release_api.rs`.

- [ ] **Step 6: Verify all three Xray hashes before returning the archive**

```rust
fn verify_xray_hashes(actual: &str, api: &str, dgst: &str) -> Result<()> {
    if actual != api || actual != dgst {
        anyhow::bail!("Xray SHA256 校验失败");
    }
    Ok(())
}

async fn verify_xray_archive(
    path: &Path,
    actual: &str,
    release: &WwpsCoreReleaseInfo,
) -> Result<()> {
    let result = verify_xray_hashes(actual, &release.api_sha256, &release.dgst_sha256);
    if result.is_err() {
        fs::remove_file(path).await.ok();
    }
    result
}
```

Use `build_asset_request(&self.asset_client, &release.download_url)?` for the archive. After stream hashing:

```rust
let actual_hash = hex::encode(hasher.finalize());
verify_xray_archive(&temp_file, &actual_hash, release).await?;
```

Delete `download_sha256_manifest`, Xray Minisign verification, and the credential-bearing `build_request`. Update UI text that displayed `release.sha256` to display `release.api_sha256`.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run: `cargo test core::system::core_upgrade::tests --all-features`

Expected: all core updater tests pass, including fixed-source and three-way hash failures.

- [ ] **Step 8: Verify removed Xray configuration and mirror surface**

Run: `rg '"(WWPS_CORE_RELEASE_MIRRORS|WWPS_CORE_RELEASE_OWNER|WWPS_CORE_RELEASE_REPO)|https://(codeberg|gitea)|fetch_json_from_mirrors|find_minisig_asset' src/core/system/core_upgrade.rs src/core/network/release_api.rs`

Expected: no matches.

- [ ] **Step 9: Commit checkpoint if explicitly authorized**

```bash
git add rust/aegis/src/core/system/core_upgrade.rs rust/aegis/src/core/xray/installer.rs
git commit -m "security: pin and verify Xray updates"
```

---

### Task 4: Security Regression Gate and Audit Closure

**Files:**
- Modify: `docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md`
- Verify: all Rust files modified in Tasks 1-3

**Interfaces:**
- Consumes: all fixed-source, request-boundary, Aegis signature, and Xray digest behavior from Tasks 1-3
- Produces: verified audit status for `AEGIS-002` and `AEGIS-003`

- [ ] **Step 1: Run source-level trust-boundary regression checks**

Run from `rust/aegis`:

```bash
rg '"(AEGIS_RELEASE_MIRRORS|AEGIS_RELEASE_REPOSITORIES|AEGIS_RELEASE_REPOSITORY|AEGIS_RELEASE_OWNER|AEGIS_RELEASE_REPO|AEGIS_RELEASE_ASSET|WWPS_CORE_RELEASE_MIRRORS|WWPS_CORE_RELEASE_OWNER|WWPS_CORE_RELEASE_REPO|NicholasDewar)|https://(codeberg\.org|gitea\.com)' src
```

Expected: no matches. Fixed Rust constant identifiers are allowed; only the quoted environment-variable names and third-party source strings are forbidden.

- [ ] **Step 2: Run formatter and focused tests**

Run:

```bash
cargo fmt --check
cargo test core::network::release_api::tests --all-features
cargo test core::crypto::minisign::tests --all-features
cargo test core::system::upgrade::tests --all-features
cargo test core::system::core_upgrade::tests --all-features
```

Expected: every command exits 0.

- [ ] **Step 3: Run the full Rust quality gate**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: both commands exit 0. If a pre-existing failure remains, report it against the recorded baseline; do not mark the audit complete and do not add lint allowances.

- [ ] **Step 4: Review the complete diff for credential and side-effect ordering**

Run:

```bash
git diff --check
git diff -- rust/aegis/src/core/network/release_api.rs rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/crypto/minisign.rs rust/aegis/src/core/system/core_upgrade.rs rust/aegis/src/core/xray/installer.rs
```

Confirm all of the following in review:

- Only `fetch_github_json` accepts a token.
- Every binary, `.minisig`, and `.dgst` request uses `build_asset_request` and the restricted asset client.
- No `self_replace`, extraction, replacement, or restart is reachable before its updater's required verification succeeds.
- Every downloaded temporary binary/archive is removed on hash or signature failure.
- Errors and logs never contain the token, Authorization header, signature contents, or query string.

- [ ] **Step 5: Update the audit only after Steps 1-4 pass**

In `AEGIS-002`, record that Aegis now fails closed on required Minisign verification while Xray uses required API/dgst/download SHA256 equality because upstream publishes no Minisign. In `AEGIS-003`, record that sources are compile-time fixed, asset redirects are exact-host validated, and credentials are API-only. Include the verification commands and date `2026-07-17`.

- [ ] **Step 6: Final review using the requesting-code-review workflow**

Review against `docs/superpowers/specs/2026-07-17-aegis-github-only-update-security-design.md`. Treat any token leak, source override, optional signature, digest fallback, unchecked redirect, or pre-verification install side effect as Critical and blocking.

- [ ] **Step 7: Commit checkpoint if explicitly authorized**

```bash
git add rust/aegis/src/core/network/release_api.rs rust/aegis/src/core/system/upgrade.rs rust/aegis/src/core/crypto/minisign.rs rust/aegis/src/core/system/core_upgrade.rs rust/aegis/src/core/xray/installer.rs docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "security: lock updater trust to GitHub"
```
