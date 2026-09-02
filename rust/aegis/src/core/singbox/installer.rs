use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

use crate::core::paths::singbox;

pub struct SingBoxInstaller;

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
        Self::replace_binary(&bin_path, singbox::BIN).await?;

        Self::create_service().await?;

        if let Err(e) = crate::core::system::maintenance::MaintenanceManager::ensure_singbox_rule_sets().await {
            log::warn!("获取 sing-box 规则集失败（可稍后通过定时任务补齐）: {}", e);
        }

        let _ = fs::remove_dir_all(temp_dir).await;

        Ok(())
    }

    /// 原子替换 sing-box 可执行文件。
    ///
    /// # 背景
    /// 不能直接用 `fs::copy(source, dest)` 覆盖可执行文件：当 `dest` 是当前正在运行的
    /// `wwps-box.service` 二进制时，`fs::copy` 会用 `open(O_WRONLY|O_CREAT|O_TRUNC)`
    /// 打开目标，而对正在被映射执行（mapped-for-execution）的 ELF 会返回
    /// `ETXTBSY`(errno 26, "Text file busy")，导致升级失败（此前表现为
    /// "复制 sing-box 二进制失败"）。
    ///
    /// 正确做法是复制到临时暂存文件 `.new`（新 inode，不受 ETXTBSY 影响），再通过
    /// `fs::rename` 原子替换。`rename(2)` 只交换目录项、不打开目标写入，因此对
    /// 正在运行的二进制永远安全。此模式对齐仓库里已验证可用的
    /// `WwpsCoreUpgradeManager::replace_core`。
    pub(crate) async fn replace_binary<S: AsRef<Path>, D: AsRef<Path>>(
        source: S,
        dest: D,
    ) -> Result<()> {
        let source = source.as_ref();
        let dest = dest.as_ref();
        if !source.exists() {
            return Err(anyhow::anyhow!(
                "未找到待替换的 sing-box 二进制: {}",
                source.display()
            ));
        }

        // 1. 复制到 `.new` 暂存文件（全新 inode，规避对正在运行二进制的 ETXTBSY）。
        let staging = dest.with_extension(
            dest.extension()
                .map(|e| format!("{}.new", e.to_string_lossy()))
                .unwrap_or_else(|| "new".into()),
        );
        fs::copy(source, &staging).await.with_context(|| {
            format!(
                "复制新 sing-box 二进制到暂存文件失败: {}",
                staging.display()
            )
        })?;

        // 2. 赋予可执行权限。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&staging).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&staging, perms)
                .await
                .context("设置 sing-box 可执行权限失败")?;
        }

        // 3. 原子替换（rename 对正在运行的二进制安全，不会触发 ETXTBSY）。
        fs::rename(&staging, dest).await.with_context(|| {
            format!(
                "原子替换 sing-box 二进制失败: {} -> {}",
                staging.display(),
                dest.display()
            )
        })?;

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

    pub(crate) fn detect_arch() -> Result<&'static str> {
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
                "https://api.github.com/repos/SagerNet/sing-box/releases?per_page=20",
            ])
            .output()
            .await
            .context("获取版本信息失败")?;

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("解析版本信息失败")?;

        Self::find_prerelease_tag(&json).ok_or_else(|| anyhow::anyhow!("未找到预发行版本"))
    }

    /// Find the newest prerelease tag (version without leading `v`) in a
    /// GitHub `releases?per_page=N` JSON array.
    fn find_prerelease_tag(json: &serde_json::Value) -> Option<String> {
        let releases = json.as_array()?;
        for release in releases {
            let prerelease = release
                .get("prerelease")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if prerelease {
                return release
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_start_matches('v').to_string());
            }
        }
        None
    }

    pub(crate) async fn download_file(url: &str, path: &str) -> Result<()> {
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

    pub(crate) async fn extract_archive(archive: &str, dest: &str) -> Result<()> {
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
    use serde_json::json;

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
    #[test]
    fn test_find_prerelease_tag_picks_first_prerelease() {
        let json = json!([
            { "tag_name": "v1.13.20", "prerelease": false },
            { "tag_name": "v1.14.0-rc.4", "prerelease": true },
            { "tag_name": "v1.14.0-rc.2", "prerelease": true }
        ]);
        assert_eq!(
            SingBoxInstaller::find_prerelease_tag(&json),
            Some("1.14.0-rc.4".to_string())
        );
    }

    #[test]
    fn test_find_prerelease_tag_none_when_no_prerelease() {
        let json = json!([{ "tag_name": "v1.13.20", "prerelease": false }]);
        assert_eq!(SingBoxInstaller::find_prerelease_tag(&json), None);
    }

    #[test]
    fn test_find_prerelease_tag_empty_or_non_array() {
        assert_eq!(SingBoxInstaller::find_prerelease_tag(&json!([])), None);
        assert_eq!(SingBoxInstaller::find_prerelease_tag(&json!({})), None);
    }

    // ---- 回归测试：原子替换二进制（修复 ETXTBSY / "复制 sing-box 二进制失败"）----

    /// `replace_binary` 应通过“.new 暂存 + rename”替换目标，内容与源一致，
    /// 目标权限为可执行，且不留 `.new` 残留。这是对旧实现 `fs::copy` 直接覆盖
    /// 正在运行二进制（会得到 ETXTBSY）的回归防护。
    #[tokio::test]
    async fn test_replace_binary_atomically_replaces_dest() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sing-box-source");
        let dest = dir.path().join("wwps-box");

        // 源为一个伪二进制内容；目标已存在（模拟“正在运行的二进制”被替换）。
        let payload: Vec<u8> = (0u8..=255).cycle().take(1024 * 1024).collect(); // 1MiB
        tokio::fs::write(&source, &payload).await.unwrap();
        tokio::fs::write(&dest, b"old-binary").await.unwrap();

        SingBoxInstaller::replace_binary(&source, &dest)
            .await
            .expect("replace_binary should succeed");

        // 目标内容 == 源内容。
        let replaced = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(replaced, payload, "目标应被原子替换为源的内容");

        // 不留暂存文件。
        assert!(
            !dir.path().join("wwps-box.new").exists(),
            "不应残留 .new 暂存文件"
        );

        // 目标应为可执行（Unix 下 0o755）。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = tokio::fs::metadata(&dest).await.unwrap().permissions();
            assert_ne!(perms.mode() & 0o111, 0, "目标应具备可执行权限");
        }

        // 源保持不变。
        assert_eq!(tokio::fs::read(&source).await.unwrap(), payload);
    }

    /// 当源不存在时返回明确错误，而非 PANIC。
    #[tokio::test]
    async fn test_replace_binary_errors_when_source_missing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("does-not-exist");
        let dest = dir.path().join("wwps-box");
        let err = SingBoxInstaller::replace_binary(&source, &dest)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("未找到待替换"),
            "缺源时应给出明确错误，got: {}",
            err
        );
    }

    /// 目标为带扩展名的路径时 `.new` 暂存命名同样合法且能正常替换。
    #[tokio::test]
    async fn test_replace_binary_handles_dotted_dest() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("bin-source");
        let dest = dir.path().join("wwps-box.service-bin");
        tokio::fs::write(&source, b"payload").await.unwrap();
        tokio::fs::write(&dest, b"old").await.unwrap();

        SingBoxInstaller::replace_binary(&source, &dest)
            .await
            .expect("replace_binary should succeed");

        assert_eq!(tokio::fs::read(&dest).await.unwrap(), b"payload");
        // 无残留暂存。
        assert!(!dest.with_extension("service-bin.new").exists());
    }
}
