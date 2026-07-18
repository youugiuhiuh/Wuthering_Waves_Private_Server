use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

use crate::core::network::release_api::{
    ReleaseResponse, fetch_github_json, find_named_asset, parse_digest,
};
use crate::core::paths::singbox;

pub struct SingBoxInstaller;

const OWNER: &str = "SagerNet";
const REPO: &str = "sing-box";
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
#[allow(dead_code)]
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

impl SingBoxInstaller {
    pub async fn is_installed() -> bool {
        fs::try_exists(singbox::BIN).await.unwrap_or(false)
    }

    pub async fn install() -> Result<()> {
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

        let arch = Self::detect_arch()?;

        let version = Self::fetch_latest_version().await?;
        let download_url = format!(
            "https://github.com/SagerNet/sing-box/releases/download/v{}/sing-box-{}-linux-{}.tar.gz",
            version, version, arch
        );

        fs::create_dir_all(singbox::DIR)
            .await
            .context("创建安装目录失败")?;
        fs::create_dir_all(singbox::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let temp_dir = "/tmp/sing-box-install";
        fs::create_dir_all(temp_dir).await?;

        let archive_path = format!("{}/sing-box.tar.gz", temp_dir);
        Self::download_file(&download_url, &archive_path).await?;

        Self::extract_archive(&archive_path, temp_dir).await?;

        let bin_path = format!("{}/sing-box-{}-linux-{}/sing-box", temp_dir, version, arch);
        fs::copy(&bin_path, singbox::BIN)
            .await
            .context("复制二进制文件失败")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(singbox::BIN).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(singbox::BIN, perms).await?;
        }

        Self::create_service().await?;

        let _ = fs::remove_dir_all(temp_dir).await;

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

    async fn fetch_latest_version() -> Result<String> {
        let output = tokio::process::Command::new("curl")
            .args([
                "-s",
                "https://api.github.com/repos/SagerNet/sing-box/releases/latest",
            ])
            .output()
            .await
            .context("获取版本信息失败")?;

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("解析版本信息失败")?;

        let tag_name = json["tag_name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("未找到版本标签"))?;

        Ok(tag_name.trim_start_matches('v').to_string())
    }

    async fn download_file(url: &str, path: &str) -> Result<()> {
        let output = tokio::process::Command::new("curl")
            .args(["-L", "-o", path, url])
            .output()
            .await
            .context("下载文件失败")?;

        if !output.status.success() {
            anyhow::bail!("下载失败: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(())
    }

    async fn extract_archive(archive: &str, dest: &str) -> Result<()> {
        let output = tokio::process::Command::new("tar")
            .args(["-xzf", archive, "-C", dest])
            .output()
            .await
            .context("解压文件失败")?;

        if !output.status.success() {
            anyhow::bail!("解压失败: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
