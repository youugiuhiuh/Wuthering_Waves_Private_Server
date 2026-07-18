use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};
use tokio::fs;

use crate::core::network::release_api::{
    ReleaseResponse, fetch_github_json, find_named_asset, parse_digest,
};
use crate::core::paths::singbox;

pub struct SingBoxInstaller;

const OWNER: &str = "SagerNet";
const REPO: &str = "sing-box";
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;

fn release_path() -> String {
    format!("repos/{OWNER}/{REPO}/releases/latest")
}

fn asset_name(version: &str, arch: &str) -> String {
    format!("sing-box-{version}-linux-{arch}.tar.gz")
}

pub struct SingBoxRelease {
    pub tag: String,
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

impl SingBoxRelease {
    pub fn from_release_response(release: &ReleaseResponse, arch: &str) -> Result<Self> {
        if release.tag_name.is_empty() {
            anyhow::bail!("Release tag_name is empty");
        }
        let version = release.tag_name.trim_start_matches('v').to_string();
        let expected_asset = asset_name(&version, arch);

        let asset = find_named_asset(&release.assets, &expected_asset)
            .ok_or_else(|| anyhow::anyhow!("Asset not found: {expected_asset}"))?;

        let download_url = asset.download_url().to_string();
        if download_url.is_empty() {
            anyhow::bail!("Asset browser_download_url is empty");
        }

        let size = asset
            .size
            .ok_or_else(|| anyhow::anyhow!("Asset size is missing"))?;
        if size > MAX_ARCHIVE_BYTES {
            anyhow::bail!("Asset size {size} exceeds max {MAX_ARCHIVE_BYTES}");
        }

        let sha256 = asset.digest.as_deref().and_then(parse_digest);

        Ok(SingBoxRelease {
            tag: release.tag_name.clone(),
            version,
            asset_name: expected_asset,
            download_url,
            sha256,
            size: Some(size),
        })
    }
}

pub async fn fetch_release(
    api_client: &reqwest::Client,
    token: Option<&str>,
    arch: &str,
) -> Result<SingBoxRelease> {
    let release = fetch_github_json::<ReleaseResponse>(api_client, &release_path(), token).await?;
    SingBoxRelease::from_release_response(&release, arch)
}

pub async fn download_verified_archive(
    client: &reqwest::Client,
    release: &SingBoxRelease,
    dir: &Path,
) -> Result<PathBuf> {
    use futures_util::StreamExt;
    use sha2::Digest;
    use tokio::io::AsyncWriteExt;

    let response = client
        .get(&release.download_url)
        .send()
        .await
        .context("Sing-box download request failed")?
        .error_for_status()
        .context("Sing-box download returned error status")?;

    let declared = response
        .content_length()
        .context("Sing-box response missing Content-Length")?;
    let expected_size = release.size.context("release missing size")?;
    if declared != expected_size || declared > MAX_ARCHIVE_BYTES {
        anyhow::bail!(
            "Sing-box archive declared size {declared} does not match expected size {expected_size} or exceeds limit {MAX_ARCHIVE_BYTES}"
        );
    }

    let archive_path = dir.join("sing-box.tar.gz");
    let mut file = fs::File::create(&archive_path).await?;

    let mut stream = response.bytes_stream();
    let mut hasher = sha2::Sha256::new();
    let mut downloaded = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Sing-box download chunk error")?;
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .context("Sing-box download size overflow")?;
        if downloaded > MAX_ARCHIVE_BYTES {
            anyhow::bail!("Sing-box archive exceeds stream limit");
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .context("Sing-box archive write failed")?;
    }

    file.sync_all()
        .await
        .context("Sing-box archive sync failed")?;

    if downloaded != expected_size {
        anyhow::bail!(
            "Sing-box archive size mismatch: expected {expected_size}, downloaded {downloaded}"
        );
    }

    if let Some(ref expected_sha256) = release.sha256 {
        let actual = hex::encode(hasher.finalize());
        if actual != *expected_sha256 {
            anyhow::bail!("Sing-box SHA256 mismatch: expected {expected_sha256}, got {actual}");
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&archive_path)
            .await
            .context("Sing-box archive metadata failed")?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&archive_path, perms)
            .await
            .context("Sing-box archive chmod failed")?;
    }

    Ok(archive_path)
}

impl SingBoxInstaller {
    pub async fn is_installed() -> bool {
        fs::try_exists(singbox::BIN).await.unwrap_or(false)
    }

    pub async fn install() -> Result<()> {
        let arch = Self::detect_arch()?;

        let api_client = reqwest::Client::new();
        let release = fetch_release(&api_client, None, arch).await?;

        fs::create_dir_all(singbox::DIR)
            .await
            .context("创建安装目录失败")?;
        fs::create_dir_all(singbox::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let temp = tempfile::Builder::new()
            .prefix("singbox-install-")
            .tempdir()
            .context("创建临时目录失败")?;
        let temp_dir = temp.path();

        let archive_path =
            download_verified_archive(&api_client, &release, temp_dir).await?;
        let candidate = extract_candidate(&archive_path, temp_dir, &release)?;
        Self::deploy_candidate(&candidate, &release, Path::new(singbox::BIN)).await?;

        let old_service_path = "/etc/systemd/system/sing-box.service";
        if tokio::fs::try_exists(old_service_path)
            .await
            .unwrap_or(false)
        {
            let _ = tokio::process::Command::new("systemctl")
                .args(["stop", "sing-box"])
                .output()
                .await;
            let _ = tokio::fs::remove_file(old_service_path).await;
            let _ = tokio::process::Command::new("systemctl")
                .args(["daemon-reload"])
                .output()
                .await;
        }

        Self::create_service().await?;

        // TempDir auto-cleans on drop — no explicit remove_dir_all needed
        let _ = temp;

        Ok(())
    }

    async fn deploy_candidate(
        candidate: &Path,
        release: &SingBoxRelease,
        dest: &Path,
    ) -> Result<()> {
        let output = tokio::process::Command::new(candidate)
            .arg("version")
            .output()
            .await
            .context("failed to execute sing-box version")?;
        let reported = parse_singbox_version(&output.stdout)?;
        if !output.status.success() || reported != release.version {
            anyhow::bail!("Sing-box candidate version mismatch");
        }

        let staged = dest.with_extension("new");
        fs::copy(candidate, &staged)
            .await
            .context("failed to stage candidate")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).await?;
        }
        let file = std::fs::File::open(&staged)?;
        file.sync_all()?;
        fs::rename(&staged, dest)
            .await
            .context("failed to rename candidate into place")?;
        Ok(())
    }

    pub async fn uninstall() -> Result<()> {
        Self::stop_service().await?;

        let _ = fs::remove_file("/etc/systemd/system/wwps-box.service").await;
        let _ = fs::remove_dir_all(singbox::DIR).await;

        Ok(())
    }

    pub async fn restart_service() -> Result<()> {
        Self::reload_service().await
    }

    pub async fn status() -> Result<String> {
        let is_installed = Self::is_installed().await;

        if !is_installed {
            return Ok("⚠️ <b>Sing-box 状态</b>: 未安装".to_string());
        }

        let running = tokio::process::Command::new("pgrep")
            .args(["-x", "wwps-box"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        Ok(format!(
            "⚙️ <b>Sing-box 状态</b>: {}",
            if running {
                "🟢 运行中"
            } else {
                "🔴 未运行"
            }
        ))
    }

    fn detect_arch() -> Result<&'static str> {
        let arch = std::env::consts::ARCH;
        Self::detect_arch_for(arch)
    }

    pub fn detect_arch_for(arch: &str) -> Result<&'static str> {
        match arch {
            "x86_64" => Ok("amd64"),
            "aarch64" => Ok("arm64"),
            "armv7l" => Ok("armv7"),
            _ => anyhow::bail!("不支持的架构: {}", arch),
        }
    }

    async fn create_service() -> Result<()> {
        if !Path::new("/run/systemd/system").exists() {
            return Ok(());
        }

        let service_content = r#"[Unit]
Description=WWPS-Box Service
After=network.target

[Service]
Type=simple
ExecStart=/etc/wwps/wwps-box/wwps-box run -C /etc/wwps/wwps-box/conf
Restart=always
RestartSec=5
LimitNOFILE=51200

[Install]
WantedBy=multi-user.target
"#;

        fs::write("/etc/systemd/system/wwps-box.service", service_content)
            .await
            .context("创建服务文件失败")?;

        if let Err(e) = crate::core::singbox::SingBoxConfigManager::ensure_base_config().await {
            log::warn!("创建基础配置失败: {}", e);
        }

        tokio::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .output()
            .await?;

        tokio::process::Command::new("systemctl")
            .args(["enable", "--now", "wwps-box"])
            .output()
            .await?;

        Ok(())
    }

    async fn stop_service() -> Result<()> {
        let _ = tokio::process::Command::new("systemctl")
            .args(["stop", "wwps-box"])
            .output()
            .await;

        Ok(())
    }

    async fn reload_service() -> Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", "wwps-box"])
            .output()
            .await
            .context("重启服务失败")?;

        if !output.status.success() {
            anyhow::bail!("重启服务失败: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(())
    }
}

pub fn parse_singbox_version(stdout: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(stdout).context("sing-box version output is not valid UTF-8")?;
    let version_token = text
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
        .ok_or_else(|| anyhow::anyhow!("no semver token found in sing-box version output"))?;
    Ok(version_token.to_string())
}

pub fn extract_candidate(
    archive: &Path,
    output_dir: &Path,
    release: &SingBoxRelease,
) -> Result<PathBuf> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let expected_dir = release
        .asset_name
        .strip_suffix(".tar.gz")
        .ok_or_else(|| anyhow::anyhow!("asset_name does not end with .tar.gz"))?;
    let expected_path_string = format!("{expected_dir}/sing-box");
    let expected_path = Path::new(&expected_path_string);

    let file = std::fs::File::open(archive)
        .with_context(|| format!("failed to open archive: {}", archive.display()))?;
    let decoder = GzDecoder::new(file);
    let mut tar = Archive::new(decoder);

    let candidate_name = output_dir.join("sing-box-candidate");
    let mut candidate = None::<PathBuf>;
    let mut expanded = 0u64;

    for entry in tar.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.into_owned();

        if entry_path.is_absolute()
            || entry_path
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            anyhow::bail!("unsafe Sing-box archive path: {}", entry_path.display());
        }

        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            anyhow::bail!(
                "unsupported Sing-box archive entry: {}",
                entry_path.display()
            );
        }

        let size = entry.size();
        expanded = expanded
            .checked_add(size)
            .context("expanded size overflow")?;
        if expanded > MAX_EXPANDED_BYTES {
            anyhow::bail!("expanded archive exceeds limit");
        }

        if entry_path == expected_path {
            if candidate.is_some() {
                anyhow::bail!("duplicate Sing-box candidate");
            }
            entry.unpack(&candidate_name)?;
            candidate = Some(candidate_name.clone());
        } else if entry_path.file_name().is_some_and(|n| n == "sing-box") {
            anyhow::bail!("unexpected Sing-box binary path: {}", entry_path.display());
        }
    }

    let candidate =
        candidate.ok_or_else(|| anyhow::anyhow!("Sing-box binary not found in archive"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&candidate)
            .context("failed to read candidate metadata")?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&candidate, perms).context("failed to set candidate mode 0755")?;
    }

    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use sha2::Digest;
    use std::io::Read;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_release(url: &str, size: u64, sha256: &str) -> SingBoxRelease {
        SingBoxRelease {
            tag: "v1.14.0".into(),
            version: "1.14.0".into(),
            asset_name: "sing-box-1.14.0-linux-amd64.tar.gz".into(),
            download_url: url.into(),
            sha256: Some(sha256.into()),
            size: Some(size),
        }
    }

    async fn mock_server(body: &[u8]) -> (MockServer, String) {
        let srv = MockServer::start().await;
        let url = format!("{}/archive", srv.uri());
        let tmpl =
            ResponseTemplate::new(200).set_body_raw(body.to_vec(), "application/octet-stream");
        Mock::given(method("GET"))
            .and(path("/archive"))
            .respond_with(tmpl)
            .mount(&srv)
            .await;
        (srv, url)
    }

    #[tokio::test]
    async fn test_download_verified_archive_success() {
        let content = b"hello sing-box archive data";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let (_srv, url) = mock_server(content).await;
        let release = make_release(&url, content.len() as u64, &hash);
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();

        let archive = download_verified_archive(&reqwest::Client::new(), &release, dir.path())
            .await
            .expect("download should succeed");
        assert_eq!(archive.file_name().unwrap(), "sing-box.tar.gz");
        assert_eq!(tokio::fs::read(&archive).await.unwrap(), content);
    }

    #[tokio::test]
    async fn test_download_rejects_metadata_size_mismatch() {
        let content = b"data";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let (srv, url) = mock_server(content).await;
        let release = make_release(&url, 9999, &hash);
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();

        let err = download_verified_archive(&reqwest::Client::new(), &release, dir.path())
            .await
            .expect_err("should reject size metadata mismatch");
        assert!(
            err.to_string().contains("size") || err.to_string().contains("expected"),
            "got: {err}"
        );
        assert_eq!(
            srv.received_requests().await.unwrap_or_default().len(),
            1,
            "should not retry after failure"
        );
        // ponytail: old binary (installed sing-box) is never touched by a temp-dir download
    }

    #[tokio::test]
    async fn test_download_rejects_hash_mismatch() {
        let content = b"real data";
        let wrong_hash = hex::encode(sha2::Sha256::digest(b"different data"));
        let (srv, url) = mock_server(content).await;
        let release = make_release(&url, content.len() as u64, &wrong_hash);
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();

        let err = download_verified_archive(&reqwest::Client::new(), &release, dir.path())
            .await
            .expect_err("should reject hash mismatch");
        assert!(err.to_string().contains("SHA256"), "got: {err}");
        assert_eq!(
            srv.received_requests().await.unwrap_or_default().len(),
            1,
            "should not retry after failure"
        );
    }

    #[tokio::test]
    async fn test_download_rejects_oversized_content_length() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/archive", addr);
        let oversized = MAX_ARCHIVE_BYTES + 1;

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", oversized);
            let _ = socket.write_all(response.as_bytes()).await;
            // keep socket alive briefly so reqwest can read headers
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        });

        let release = make_release(&url, 123, "");
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();

        let err = download_verified_archive(&reqwest::Client::new(), &release, dir.path())
            .await
            .expect_err("should reject oversized Content-Length");
        assert!(
            err.to_string().contains("exceeds")
                || err.to_string().contains("limit")
                || err.to_string().contains("max"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_download_sets_file_mode_0600() {
        let content = b"mode check data";
        let hash = hex::encode(sha2::Sha256::digest(content));
        let (_srv, url) = mock_server(content).await;
        let release = make_release(&url, content.len() as u64, &hash);
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();

        let archive = download_verified_archive(&reqwest::Client::new(), &release, dir.path())
            .await
            .expect("download should succeed");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = tokio::fs::metadata(&archive).await.unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "expected 0600 mode, got {:#o}", mode);
        }
        #[cfg(not(unix))]
        let _ = archive;
    }

    #[test]
    fn singbox_asset_identity_is_exact() {
        assert_eq!(release_path(), "repos/SagerNet/sing-box/releases/latest");
        assert_eq!(
            asset_name("1.13.14", "amd64"),
            "sing-box-1.13.14-linux-amd64.tar.gz"
        );
    }

    #[test]
    fn singbox_constants_are_correct() {
        assert_eq!(OWNER, "SagerNet");
        assert_eq!(REPO, "sing-box");
        assert_eq!(MAX_ARCHIVE_BYTES, 128 * 1024 * 1024);
        assert_eq!(MAX_EXPANDED_BYTES, 256 * 1024 * 1024);
    }

    #[test]
    fn singbox_release_from_response() {
        let json = r#"{
            "tag_name": "v1.14.0",
            "assets": [{
                "name": "sing-box-1.14.0-linux-amd64.tar.gz",
                "browser_download_url": "https://github.com/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-amd64.tar.gz",
                "size": 12345678,
                "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }]
        }"#;
        let release: ReleaseResponse = serde_json::from_str(json).unwrap();
        let sb = SingBoxRelease::from_release_response(&release, "amd64").unwrap();
        assert_eq!(sb.tag, "v1.14.0");
        assert_eq!(sb.version, "1.14.0");
        assert_eq!(sb.asset_name, "sing-box-1.14.0-linux-amd64.tar.gz");
        assert_eq!(
            sb.download_url,
            "https://github.com/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-amd64.tar.gz"
        );
        assert_eq!(
            sb.sha256,
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into())
        );
        assert_eq!(sb.size, Some(12345678));
    }

    #[test]
    fn singbox_release_rejects_missing_asset() {
        let json = r#"{"tag_name": "v1.14.0", "assets": []}"#;
        let release: ReleaseResponse = serde_json::from_str(json).unwrap();
        assert!(SingBoxRelease::from_release_response(&release, "amd64").is_err());
    }

    #[test]
    fn singbox_release_rejects_empty_download_url() {
        let json = r#"{
            "tag_name": "v1.14.0",
            "assets": [{
                "name": "sing-box-1.14.0-linux-amd64.tar.gz",
                "browser_download_url": "",
                "size": 12345678
            }]
        }"#;
        let release: ReleaseResponse = serde_json::from_str(json).unwrap();
        assert!(SingBoxRelease::from_release_response(&release, "amd64").is_err());
    }

    #[test]
    fn singbox_release_rejects_missing_size() {
        let json = r#"{
            "tag_name": "v1.14.0",
            "assets": [{
                "name": "sing-box-1.14.0-linux-amd64.tar.gz",
                "browser_download_url": "https://github.com/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-amd64.tar.gz"
            }]
        }"#;
        let release: ReleaseResponse = serde_json::from_str(json).unwrap();
        assert!(SingBoxRelease::from_release_response(&release, "amd64").is_err());
    }

    #[test]
    fn singbox_release_rejects_size_exceeds_max() {
        let json = format!(
            r#"{{
                "tag_name": "v1.14.0",
                "assets": [{{
                    "name": "sing-box-1.14.0-linux-amd64.tar.gz",
                    "browser_download_url": "https://github.com/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-amd64.tar.gz",
                    "size": {}
                }}]
            }}"#,
            MAX_ARCHIVE_BYTES + 1
        );
        let release: ReleaseResponse = serde_json::from_str(&json).unwrap();
        assert!(SingBoxRelease::from_release_response(&release, "amd64").is_err());
    }

    #[test]
    fn test_detect_arch_x86_64() {
        let result = SingBoxInstaller::detect_arch_for("x86_64").unwrap();
        assert_eq!(result, "amd64");
    }

    #[test]
    fn test_detect_arch_aarch64() {
        let result = SingBoxInstaller::detect_arch_for("aarch64").unwrap();
        assert_eq!(result, "arm64");
    }

    #[test]
    fn test_detect_arch_armv7l() {
        let result = SingBoxInstaller::detect_arch_for("armv7l").unwrap();
        assert_eq!(result, "armv7");
    }

    #[test]
    fn test_detect_arch_unsupported() {
        let result = SingBoxInstaller::detect_arch_for("unsupported");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "不支持的架构: unsupported");
    }

    #[test]
    fn test_detect_arch_s390x() {
        let result = SingBoxInstaller::detect_arch_for("s390x");
        assert!(result.is_err());
    }

    fn make_tar_gz<F>(f: F) -> Vec<u8>
    where
        F: FnOnce(&mut tar::Builder<GzEncoder<Vec<u8>>>),
    {
        let encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        f(&mut archive);
        let encoder = archive.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    fn make_tar_gz_with_path(path: &str, content: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let placeholder = "x".repeat(path.len());
        let mut raw = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(content.len() as u64);
            builder.append_data(&mut h, &placeholder, content).unwrap();
            builder.finish().unwrap();
        }
        let pb = path.as_bytes();
        let ph = placeholder.as_bytes();
        if let Some(pos) = raw.windows(ph.len()).position(|w| w == ph) {
            raw[pos..pos + ph.len()].copy_from_slice(pb);
        }
        for i in 148..156 {
            raw[i] = b' ';
        }
        let sum: u32 = raw[..512].iter().map(|&b| b as u32).sum();
        let _ = write!(&mut raw[148..155], "{:06o}", sum);
        raw[154] = b' ';
        raw[155] = b' ';
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap()
    }

    fn test_release() -> SingBoxRelease {
        SingBoxRelease {
            tag: "v1.14.0".into(),
            version: "1.14.0".into(),
            asset_name: "sing-box-1.14.0-linux-amd64.tar.gz".into(),
            download_url: String::new(),
            sha256: None,
            size: None,
        }
    }

    #[test]
    fn test_extract_candidate_success() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            tar.append_dir("sing-box-1.14.0-linux-amd64", ".").unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o755);
            h.set_size(5);
            tar.append_data(
                &mut h,
                "sing-box-1.14.0-linux-amd64/sing-box",
                "hello".as_bytes(),
            )
            .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let result = extract_candidate(&archive, dir.path(), &release).unwrap();
        assert_eq!(result, dir.path().join("sing-box-candidate"));
        assert_eq!(std::fs::read(&result).unwrap(), b"hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&result).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "expected 0755, got {:#o}", mode);
        }
    }

    #[test]
    fn test_extract_rejects_absolute_path() {
        let release = test_release();
        let data = make_tar_gz_with_path("/etc/passwd", b"root");
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unsafe"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_parent_dir() {
        let release = test_release();
        let data = make_tar_gz_with_path("../evil", b"data");
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unsafe"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_symlink() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            tar.append_data(&mut h, "evil-link", std::io::empty())
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unsupported"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_hardlink() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Link);
            h.set_size(0);
            tar.append_data(&mut h, "evil-hardlink", std::io::empty())
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unsupported"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_device() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Char);
            h.set_size(0);
            tar.append_data(&mut h, "dev/tty", std::io::empty())
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unsupported"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_duplicate_binary() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            tar.append_dir("sing-box-1.14.0-linux-amd64", ".").unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(4);
            tar.append_data(
                &mut h,
                "sing-box-1.14.0-linux-amd64/sing-box",
                "bin1".as_bytes(),
            )
            .unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(4);
            tar.append_data(
                &mut h,
                "sing-box-1.14.0-linux-amd64/sing-box",
                "bin2".as_bytes(),
            )
            .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("duplicate"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_unexpected_binary() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            tar.append_dir("other-dir", ".").unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(4);
            tar.append_data(&mut h, "other-dir/sing-box", "data".as_bytes())
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("unexpected"), "got: {err}");
    }

    #[test]
    fn test_extract_rejects_expanded_overflow() {
        let release = test_release();
        let oversized = MAX_EXPANDED_BYTES + 1;
        let data = make_tar_gz(|tar| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(oversized);
            tar.append_data(&mut h, "huge-file", std::io::repeat(0).take(oversized))
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(
            err.to_string().contains("exceeds") || err.to_string().contains("limit"),
            "got: {err}"
        );
    }

    #[test]
    fn test_extract_rejects_missing_candidate() {
        let release = test_release();
        let data = make_tar_gz(|tar| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(4);
            tar.append_data(&mut h, "some-other-file", "data".as_bytes())
                .unwrap();
        });
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let archive = dir.path().join("archive.tar.gz");
        std::fs::write(&archive, &data).unwrap();
        let err = extract_candidate(&archive, dir.path(), &release).unwrap_err();
        assert!(err.to_string().contains("not found"), "got: {err}");
    }

    // -- task 4: version gate and atomic install tests --

    fn create_fake_candidate(dir: &Path, version: &str) -> PathBuf {
        let path = dir.join("fake-sing-box");
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho 'sing-box version {version}'\n"),
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        path
    }

    #[test]
    fn test_parse_singbox_version_success() {
        assert_eq!(
            parse_singbox_version(b"sing-box version 1.14.0\n").unwrap(),
            "1.14.0"
        );
    }

    #[test]
    fn test_parse_singbox_version_carriage_return() {
        assert_eq!(
            parse_singbox_version(b"sing-box version 1.14.0\r\n").unwrap(),
            "1.14.0"
        );
    }

    #[test]
    fn test_parse_singbox_version_additional_text() {
        assert_eq!(
            parse_singbox_version(b"sing-box version 1.14.0 (build 20240101)\n").unwrap(),
            "1.14.0"
        );
    }

    #[test]
    fn test_parse_singbox_version_empty() {
        let err = parse_singbox_version(b"").unwrap_err();
        assert!(err.to_string().contains("no semver"), "got: {err}");
    }

    #[test]
    fn test_parse_singbox_version_no_version_token() {
        let err = parse_singbox_version(b"sing-box version\n").unwrap_err();
        assert!(err.to_string().contains("no semver"), "got: {err}");
    }

    #[test]
    fn test_parse_singbox_version_garbage() {
        let err = parse_singbox_version(b"!!!\n").unwrap_err();
        assert!(err.to_string().contains("no semver"), "got: {err}");
    }

    #[test]
    fn test_parse_singbox_version_not_utf8() {
        let err = parse_singbox_version(b"\xff\xfe").unwrap_err();
        assert!(err.to_string().contains("UTF-8"), "got: {err}");
    }

    #[tokio::test]
    async fn test_deploy_candidate_accepts_matching_version() {
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let candidate = create_fake_candidate(dir.path(), "1.14.0");
        let target = dir.path().join("wwps-box");
        std::fs::write(&target, b"old content").unwrap();

        let release = test_release();
        SingBoxInstaller::deploy_candidate(&candidate, &release, &target)
            .await
            .expect("deploy should succeed");

        let content = std::fs::read(&target).unwrap();
        assert!(
            String::from_utf8_lossy(&content).contains("1.14.0"),
            "target should contain candidate content"
        );

        let staged = dir.path().join("wwps-box.new");
        assert!(
            !staged.exists(),
            "staged file should be removed after rename"
        );
    }

    #[tokio::test]
    async fn test_deploy_candidate_rejects_version_mismatch() {
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let candidate = create_fake_candidate(dir.path(), "9.99.99");
        let target = dir.path().join("wwps-box");
        let old = b"unalterable binary content";
        std::fs::write(&target, old).unwrap();

        let release = test_release();
        let err = SingBoxInstaller::deploy_candidate(&candidate, &release, &target)
            .await
            .expect_err("should reject version mismatch");
        assert!(
            err.to_string().contains("mismatch")
                || (err.to_string().contains("expected") && err.to_string().contains("got")),
            "got: {err}"
        );

        assert_eq!(std::fs::read(&target).unwrap(), old, "old binary preserved");
    }

    #[tokio::test]
    async fn test_deploy_candidate_rejects_execution_failure() {
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let candidate = dir.path().join("fake-sing-box");
        std::fs::write(&candidate, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            &candidate,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();

        let target = dir.path().join("wwps-box");
        let old = b"preserved binary";
        std::fs::write(&target, old).unwrap();

        let release = test_release();
        let err = SingBoxInstaller::deploy_candidate(&candidate, &release, &target)
            .await
            .expect_err("should reject execution failure");
        assert!(
            err.to_string().contains("version") || err.to_string().contains("mismatch"),
            "got: {err}"
        );

        assert_eq!(std::fs::read(&target).unwrap(), old, "old binary preserved");
    }

    #[tokio::test]
    async fn test_deploy_candidate_rejects_rename_failure() {
        let dir = tempfile::Builder::new()
            .prefix("sbtest-")
            .tempdir()
            .unwrap();
        let candidate = create_fake_candidate(dir.path(), "1.14.0");
        let target = dir.path().join("wwps-box");
        std::fs::create_dir(&target).unwrap();

        let release = test_release();
        SingBoxInstaller::deploy_candidate(&candidate, &release, &target)
            .await
            .expect_err("rename should fail when target is a directory");

        assert!(target.is_dir(), "target should remain a directory");

        let staged = dir.path().join("wwps-box.new");
        let _ = std::fs::remove_file(&staged);
    }
}
