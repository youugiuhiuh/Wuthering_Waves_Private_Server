use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

const WWPS_BOX_DIR: &str = "/etc/wwps/wwps-box";
const WWPS_BOX_BIN: &str = "/etc/wwps/wwps-box/sing-box";
const WWPS_BOX_CONF_DIR: &str = "/etc/wwps/wwps-box/conf";

pub struct SingBoxInstaller;

impl SingBoxInstaller {
    pub async fn is_installed() -> bool {
        fs::try_exists(WWPS_BOX_BIN)
            .await
            .unwrap_or(false)
    }

    pub async fn install() -> Result<()> {
        let arch = Self::detect_arch()?;

        let version = Self::fetch_latest_version().await?;
        let download_url = format!(
            "https://github.com/SagerNet/sing-box/releases/download/v{}/sing-box-{}-linux-{}.tar.gz",
            version, version, arch
        );

        fs::create_dir_all(WWPS_BOX_DIR)
            .await
            .context("创建安装目录失败")?;
        fs::create_dir_all(WWPS_BOX_CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let temp_dir = "/tmp/sing-box-install";
        fs::create_dir_all(temp_dir).await?;

        let archive_path = format!("{}/sing-box.tar.gz", temp_dir);
        Self::download_file(&download_url, &archive_path).await?;

        Self::extract_archive(&archive_path, temp_dir).await?;

        let bin_path = format!("{}/sing-box-{}/sing-box", temp_dir, version);
        fs::copy(&bin_path, WWPS_BOX_BIN)
            .await
            .context("复制二进制文件失败")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(WWPS_BOX_BIN).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(WWPS_BOX_BIN, perms).await?;
        }

        Self::create_service().await?;

        let _ = fs::remove_dir_all(temp_dir).await;

        Ok(())
    }

    pub async fn uninstall() -> Result<()> {
        Self::stop_service().await?;

        let _ = fs::remove_file("/etc/systemd/system/sing-box.service").await;
        let _ = fs::remove_dir_all(WWPS_BOX_DIR).await;

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
            .args(["-x", "sing-box"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        Ok(format!(
            "⚙️ <b>Sing-box 状态</b>: {}",
            if running { "🟢 运行中" } else { "🔴 未运行" }
        ))
    }

    fn detect_arch() -> Result<&'static str> {
        let arch = std::env::consts::ARCH;
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

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("解析版本信息失败")?;

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
Description=Sing-box Service
After=network.target

[Service]
Type=simple
ExecStart=/etc/wwps/wwps-box/sing-box run -C /etc/wwps/wwps-box/conf
Restart=always
RestartSec=5
LimitNOFILE=51200

[Install]
WantedBy=multi-user.target
"#;

        fs::write("/etc/systemd/system/sing-box.service", service_content)
            .await
            .context("创建服务文件失败")?;

        tokio::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .output()
            .await?;

        tokio::process::Command::new("systemctl")
            .args(["enable", "--now", "sing-box"])
            .output()
            .await?;

        Ok(())
    }

    async fn stop_service() -> Result<()> {
        let _ = tokio::process::Command::new("systemctl")
            .args(["stop", "sing-box"])
            .output()
            .await;

        Ok(())
    }

    async fn reload_service() -> Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", "sing-box"])
            .output()
            .await
            .context("重启服务失败")?;

        if !output.status.success() {
            anyhow::bail!(
                "重启服务失败: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }
}