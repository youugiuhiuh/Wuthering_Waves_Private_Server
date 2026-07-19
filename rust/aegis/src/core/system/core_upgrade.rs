use crate::adapters::common::{BotAdapter, MessageContent, MessageId as AegisMsgId, TargetId};
use crate::core::cmd_async::run_cmd_status;
use crate::core::network::release_api::{
    ReleaseResponse, build_asset_request, fetch_github_json, fetch_github_json_with_query,
    find_named_asset, github_api_client, github_asset_client, parse_digest, parse_xray_sha256_dgst,
};
use crate::core::paths::xray;
use crate::core::utils::{format_download_progress, human_readable_size, should_report};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use futures_util::StreamExt;
use rust_i18n::t;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{File as StdFile, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::task;
use zip::ZipArchive;

use crate::core::system::upgrade_transaction::{
    PublishedBinary, SingleFlight, activate_or_rollback, publish_binary, stage_binary,
};

const XRAY_RELEASE_OWNER: &str = "XTLS";
const XRAY_RELEASE_REPO: &str = "Xray-core";
const WWPS_CORE_DEFAULT_SERVICE: &str = xray::DEFAULT_SERVICE;
const WWPS_CORE_DEFAULT_INSTALL_DIR: &str = xray::DIR;
const WWPS_CORE_DEFAULT_TEMP_DIR: &str = xray::DEFAULT_TEMP_DIR;
const WWPS_CORE_DEFAULT_BACKUP_PREFIX: &str = xray::DEFAULT_BACKUP_PREFIX;

static WWPS_CORE_UPGRADE: SingleFlight = SingleFlight::new("wwps-core");
const HEALTH_ATTEMPTS: usize = 10;
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);
const HEALTH_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

fn xray_release_path(tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases/tags/{tag}"),
        None => format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases/latest"),
    }
}

fn xray_releases_path() -> String {
    format!("repos/{XRAY_RELEASE_OWNER}/{XRAY_RELEASE_REPO}/releases")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    Amd64,
    Arm64,
}

impl CpuArch {
    pub fn detect() -> Result<Self> {
        Self::from_arch_str(std::env::consts::ARCH)
    }

    pub fn from_arch_str(value: &str) -> Result<Self> {
        match value {
            "x86_64" | "amd64" => Ok(Self::Amd64),
            "aarch64" | "arm64" => Ok(Self::Arm64),
            other => anyhow::bail!("暂不支持的 CPU 架构: {}", other),
        }
    }

    pub fn asset_basename(&self) -> &'static str {
        match self {
            CpuArch::Amd64 => "Xray-linux-64",
            CpuArch::Arm64 => "Xray-linux-arm64-v8a",
        }
    }
}

struct TempPathGuard {
    path: PathBuf,
    is_dir: bool,
    armed: bool,
}

impl TempPathGuard {
    fn file(path: PathBuf) -> Self {
        Self {
            path,
            is_dir: false,
            armed: true,
        }
    }

    fn directory(path: PathBuf) -> Self {
        Self {
            path,
            is_dir: true,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.is_dir {
            let _ = std::fs::remove_dir_all(&self.path);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn bounded_health<F, Fut>(mut probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut last = None;
    for attempt in 0..HEALTH_ATTEMPTS {
        match probe().await {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        if attempt + 1 < HEALTH_ATTEMPTS {
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    }
    Err(last.context("health probe did not run")?).context(format!(
        "wwps-core service unhealthy after {HEALTH_ATTEMPTS} attempts"
    ))
}

#[derive(Debug, Clone)]
pub struct WwpsCoreUpgradeConfig {
    pub service_name: String,
    pub install_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub arch: CpuArch,
}

#[derive(Debug, Clone)]
pub struct WwpsCoreReleaseInfo {
    pub tag_name: String,
    pub asset_name: String,
    pub download_url: String,
    pub api_sha256: String,
    pub dgst_sha256: String,
    pub size: Option<u64>,
}

pub struct WwpsCoreUpgradeManager {
    config: Arc<WwpsCoreUpgradeConfig>,
    api_client: reqwest::Client,
    asset_client: reqwest::Client,
    github_token: Option<String>,
}

impl WwpsCoreUpgradeConfig {
    pub fn new(
        service_name: impl Into<String>,
        install_dir: PathBuf,
        backup_dir: PathBuf,
        temp_dir: PathBuf,
        arch: CpuArch,
    ) -> Self {
        Self {
            service_name: service_name.into(),
            install_dir,
            backup_dir,
            temp_dir,
            arch,
        }
    }

    pub fn from_env() -> Result<Self> {
        let service_name = env::var("WWPS_CORE_SERVICE_NAME")
            .unwrap_or_else(|_| WWPS_CORE_DEFAULT_SERVICE.to_string());

        let install_dir = env::var("WWPS_CORE_INSTALL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(WWPS_CORE_DEFAULT_INSTALL_DIR));

        let backup_dir = env::var("WWPS_CORE_BACKUP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| install_dir.join("backup"));

        let temp_dir = env::var("WWPS_CORE_TEMP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(WWPS_CORE_DEFAULT_TEMP_DIR));

        let arch = CpuArch::detect()?;

        Ok(Self::new(
            service_name,
            install_dir,
            backup_dir,
            temp_dir,
            arch,
        ))
    }

    pub fn validate(&self) -> Result<()> {
        if !self.install_dir.exists() {
            anyhow::bail!("wwps-core 安装目录不存在: {}", self.install_dir.display());
        }

        let binary_path = self.install_dir.join("wwps-core");
        if !binary_path.exists() {
            anyhow::bail!(
                "未找到 wwps-core 可执行文件，请先通过 install.sh 安装: {}",
                binary_path.display()
            );
        }

        Self::ensure_dir_writable(&self.install_dir)?;
        Self::ensure_dir_writable(&self.backup_dir)?;
        Self::ensure_dir_writable(&self.temp_dir)?;
        Ok(())
    }

    fn ensure_dir_writable(path: &Path) -> Result<()> {
        if !path.exists() {
            std::fs::create_dir_all(path)
                .with_context(|| format!("创建目录失败: {}", path.display()))?;
        }

        let test_path = path.join(format!(".write-test-{}", std::process::id()));
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        opts.open(&test_path)
            .with_context(|| format!("目录不可写: {}", path.display()))?;
        std::fs::remove_file(&test_path).ok();
        Ok(())
    }
}

impl WwpsCoreUpgradeManager {
    pub fn new(config: WwpsCoreUpgradeConfig) -> Result<Self> {
        Ok(Self {
            config: Arc::new(config),
            api_client: github_api_client(Duration::from_secs(60))?,
            asset_client: github_asset_client(Duration::from_secs(60))?,
            github_token: env::var("GITHUB_TOKEN").ok().filter(|v| !v.is_empty()),
        })
    }

    pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }

        let path = xray_releases_path();
        let per_page = limit.to_string();
        let query = [("per_page", per_page.as_str())];
        let releases: Vec<ReleaseResponse> = fetch_github_json_with_query(
            &self.api_client,
            &path,
            &query,
            self.github_token.as_deref(),
        )
        .await?;

        Ok(releases
            .into_iter()
            .map(|r| r.tag_name)
            .take(limit)
            .collect())
    }

    pub async fn fetch_release(&self, tag: Option<&str>) -> Result<WwpsCoreReleaseInfo> {
        let release: ReleaseResponse = fetch_github_json(
            &self.api_client,
            &xray_release_path(tag),
            self.github_token.as_deref(),
        )
        .await?;

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
            asset
                .digest
                .as_deref()
                .ok_or_else(|| anyhow!("Xray 资产缺少 API digest"))?,
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
    }

    pub async fn download_release(
        &self,
        release: &WwpsCoreReleaseInfo,
        adapter: Option<&dyn BotAdapter>,
        target: Option<&TargetId>,
        msg_id: Option<&AegisMsgId>,
    ) -> Result<PathBuf> {
        let temp_file = self.config.temp_dir.join(format!(
            "wwps-core-{}-{}.zip",
            release.tag_name,
            Utc::now().timestamp()
        ));
        let mut temp_cleanup = TempPathGuard::file(temp_file.clone());

        fs::create_dir_all(&self.config.temp_dir)
            .await
            .context("创建临时目录失败")?;

        let response = build_asset_request(&self.asset_client, &release.download_url)?
            .send()
            .await
            .context("下载 Xray Release 失败")?
            .error_for_status()
            .context("Xray Release 下载返回错误状态")?;

        let total_size = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = fs::File::create(&temp_file)
            .await
            .context("创建 Xray 临时包失败")?;
        let mut writer = tokio::io::BufWriter::new(&mut file);
        let mut hasher = Sha256::new();

        let mut downloaded: u64 = 0;
        let mut last_pct = 0.0;
        let mut last_size = 0;
        let mut last_instant = Instant::now();
        let start = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("下载数据块失败")?;
            hasher.update(&chunk);
            writer
                .write_all(&chunk)
                .await
                .context("写入 Xray 临时包失败")?;
            downloaded += chunk.len() as u64;

            if let (Some(adapter), Some(target), Some(msg_id)) = (adapter, target, msg_id)
                && should_report(
                    downloaded,
                    total_size,
                    &mut last_pct,
                    &mut last_size,
                    last_instant,
                )
            {
                last_instant = Instant::now();
                let progress_text = format_download_progress(downloaded, total_size, start);
                let _ = adapter
                    .edit_message(
                        target,
                        msg_id,
                        MessageContent {
                            text: progress_text,
                            markup: None,
                        },
                    )
                    .await;
            }
        }

        writer.flush().await.context("刷新 Xray 临时包失败")?;
        drop(writer);
        file.sync_all().await.context("同步 Xray 包失败")?;

        let actual_hash = hex::encode(hasher.finalize());
        verify_xray_archive(&temp_file, &actual_hash, release).await?;

        temp_cleanup.disarm();
        Ok(temp_file)
    }

    pub async fn extract_archive(&self, archive_path: &Path) -> Result<PathBuf> {
        let target = self
            .config
            .temp_dir
            .join(format!("wwps-core-unpack-{}", Utc::now().timestamp()));
        fs::create_dir_all(&target)
            .await
            .context("创建解压目录失败")?;
        let mut target_cleanup = TempPathGuard::directory(target.clone());

        let archive_path = archive_path.to_owned();
        let target_clone = target.clone();
        task::spawn_blocking(move || -> Result<()> {
            let file = StdFile::open(&archive_path)
                .with_context(|| format!("打开压缩包失败: {}", archive_path.display()))?;
            let mut archive = ZipArchive::new(file).context("读取 zip 文件失败")?;
            archive
                .extract(&target_clone)
                .context("解压 zip 文件失败")?;
            Ok(())
        })
        .await
        .context("等待解压任务失败")??;

        target_cleanup.disarm();
        Ok(target)
    }

    pub async fn backup_current_core(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.config.backup_dir)
            .await
            .context("创建备份目录失败")?;

        let backup_path = self.config.backup_dir.join(format!(
            "{}-{}",
            WWPS_CORE_DEFAULT_BACKUP_PREFIX,
            Utc::now().format("%Y%m%d%H%M%S")
        ));

        fs::create_dir_all(&backup_path)
            .await
            .context("创建备份子目录失败")?;

        let core_path = self.config.install_dir.join("wwps-core");
        let backup_core = backup_path.join("wwps-core");
        tokio::fs::copy(&core_path, &backup_core)
            .await
            .with_context(|| format!("备份 wwps-core 核心失败: {}", core_path.display()))?;

        for data in ["geoip.dat", "geosite.dat"] {
            let src = self.config.install_dir.join(data);
            if src.exists() {
                let dst = backup_path.join(data);
                let _ = tokio::fs::copy(&src, &dst).await;
            }
        }

        Ok(backup_path)
    }

    pub async fn deploy_core(&self, unpack_dir: &Path) -> Result<PublishedBinary> {
        let new_core = unpack_dir.join("xray");
        if !new_core.exists() {
            anyhow::bail!("解压目录中未找到 xray 可执行文件");
        }
        let destination = self.config.install_dir.join("wwps-core");
        let staged = stage_binary(&new_core, &destination).await?;
        publish_binary(&staged, &destination).await
    }

    pub async fn restart_service(&self) -> Result<()> {
        let unit = format!("{}.service", self.config.service_name);
        let status = run_cmd_status("systemctl", &["restart", &unit], Duration::from_secs(30))
            .await
            .context("执行 systemctl restart 失败")?;

        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("systemctl restart {} 失败", unit);
        }
    }

    pub async fn verify_service_active(&self) -> Result<()> {
        let unit = format!("{}.service", self.config.service_name);
        bounded_health(|| async {
            let status = run_cmd_status(
                "systemctl",
                &["is-active", "--quiet", &unit],
                HEALTH_COMMAND_TIMEOUT,
            )
            .await
            .context("执行 systemctl is-active 失败")?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!("{} 未在运行", unit)
            }
        })
        .await
    }

    async fn restore_prior_runtime(&self, had_original: bool) -> Result<()> {
        if !had_original {
            anyhow::bail!("wwps-core rollback has no prior binary");
        }
        self.restart_service().await
    }

    async fn verify_restored_runtime(&self, had_original: bool) -> Result<()> {
        if !had_original {
            anyhow::bail!("wwps-core rollback has no prior runtime");
        }
        self.verify_service_active().await
    }

    pub async fn cleanup_paths(&self, paths: &[PathBuf]) {
        for path in paths {
            if !path.exists() {
                continue;
            }
            let _ = if path.is_dir() {
                fs::remove_dir_all(path).await
            } else {
                fs::remove_file(path).await
            };
        }
    }

    pub async fn run_upgrade(
        tag: Option<String>,
        adapter: &dyn BotAdapter,
        target: &TargetId,
    ) -> Result<()> {
        let _flight = WWPS_CORE_UPGRADE.try_enter()?;

        let status_msg_id = adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("upgrade.core_checking").to_string(),
                    markup: None,
                },
            )
            .await?;

        let config = WwpsCoreUpgradeConfig::from_env()?;
        config.validate()?;
        let manager = WwpsCoreUpgradeManager::new(config)?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_fetching").to_string(),
                    markup: None,
                },
            )
            .await;

        let release = manager.fetch_release(tag.as_deref()).await?;

        let size_str = release
            .size
            .map(human_readable_size)
            .unwrap_or_else(|| t!("upgrade.core_unknown_size").to_string());
        let info_text = t!(
            "upgrade.core_download_info",
            "0" => release.tag_name.as_str(),
            "1" => size_str.as_str(),
            "2" => release.api_sha256.as_str()
        )
        .to_string();
        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: info_text,
                    markup: None,
                },
            )
            .await;

        let archive_path = manager
            .download_release(&release, Some(adapter), Some(target), Some(&status_msg_id))
            .await?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_extracting").to_string(),
                    markup: None,
                },
            )
            .await;
        let unpack_dir = match manager.extract_archive(&archive_path).await {
            Ok(path) => path,
            Err(error) => {
                manager
                    .cleanup_paths(std::slice::from_ref(&archive_path))
                    .await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_backing_up").to_string(),
                    markup: None,
                },
            )
            .await;
        let archival_backup = match manager.backup_current_core().await {
            Ok(path) => path,
            Err(error) => {
                manager.cleanup_paths(&[archive_path, unpack_dir]).await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_replacing").to_string(),
                    markup: None,
                },
            )
            .await;
        let published = match manager.deploy_core(&unpack_dir).await {
            Ok(value) => value,
            Err(error) => {
                manager
                    .cleanup_paths(&[archive_path, unpack_dir, archival_backup])
                    .await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_restarting").to_string(),
                    markup: None,
                },
            )
            .await;
        let activation = activate_or_rollback(
            &published,
            || manager.restart_service(),
            || manager.verify_service_active(),
            |had_original| manager.restore_prior_runtime(had_original),
            |had_original| manager.verify_restored_runtime(had_original),
        )
        .await;
        manager.cleanup_paths(&[archive_path, unpack_dir]).await;
        if let Err(error) = activation {
            manager
                .cleanup_paths(std::slice::from_ref(&archival_backup))
                .await;
            return Err(error);
        }

        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!(
                        "upgrade.core_updated",
                        "0" => release.tag_name.as_str(),
                        "1" => archival_backup.display().to_string().as_str()
                    )
                    .to_string(),
                    markup: None,
                },
            )
            .await?;
        Ok(())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::system::upgrade_transaction::rollback_binary;
    use tempfile::tempdir;

    #[test]
    fn xray_recent_releases_path_contains_no_query() {
        let path = xray_releases_path();
        assert_eq!(path, "repos/XTLS/Xray-core/releases");
        assert!(!path.contains('?'));
    }

    #[test]
    fn xray_release_identity_is_fixed() {
        assert_eq!(XRAY_RELEASE_OWNER, "XTLS");
        assert_eq!(XRAY_RELEASE_REPO, "Xray-core");
        assert_eq!(
            xray_release_path(None),
            "repos/XTLS/Xray-core/releases/latest"
        );
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
            download_url:
                "https://github.com/XTLS/Xray-core/releases/download/v26.3.27/Xray-linux-64.zip"
                    .into(),
            api_sha256: "1".repeat(64),
            dgst_sha256: "1".repeat(64),
            size: None,
        };
        assert!(
            verify_xray_archive(&archive, &"0".repeat(64), &release)
                .await
                .is_err()
        );
        assert!(!archive.exists());
    }

    #[test]
    fn test_cpu_arch_detection() {
        assert_eq!(CpuArch::from_arch_str("x86_64").unwrap(), CpuArch::Amd64);
        assert_eq!(CpuArch::from_arch_str("aarch64").unwrap(), CpuArch::Arm64);
        assert!(CpuArch::from_arch_str("mips").is_err());
    }

    #[test]
    fn test_config_validation_success() {
        let tmp = tempdir().unwrap();
        let install_dir = tmp.path().join("wwps-core-install");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(install_dir.join("wwps-core"), b"binary").unwrap();
        let backup_dir = tmp.path().join("backup");
        let temp_dir = tmp.path().join("temp");

        let config = WwpsCoreUpgradeConfig::new(
            "wwps-core",
            install_dir,
            backup_dir,
            temp_dir,
            CpuArch::Amd64,
        );

        config.validate().unwrap();
    }

    #[test]
    fn test_config_validation_missing_binary() {
        let tmp = tempdir().unwrap();
        let install_dir = tmp.path().join("wwps-core-install");
        std::fs::create_dir_all(&install_dir).unwrap();
        let backup_dir = tmp.path().join("backup");
        let temp_dir = tmp.path().join("temp");

        let config = WwpsCoreUpgradeConfig::new(
            "wwps-core",
            install_dir,
            backup_dir,
            temp_dir,
            CpuArch::Amd64,
        );

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cpu_arch_from_amd64() {
        assert_eq!(CpuArch::from_arch_str("amd64").unwrap(), CpuArch::Amd64);
    }

    #[test]
    fn test_cpu_arch_from_arm64() {
        assert_eq!(CpuArch::from_arch_str("arm64").unwrap(), CpuArch::Arm64);
    }

    #[test]
    fn test_cpu_arch_equality() {
        assert_eq!(CpuArch::Amd64, CpuArch::Amd64);
        assert_eq!(CpuArch::Arm64, CpuArch::Arm64);
        assert_ne!(CpuArch::Amd64, CpuArch::Arm64);
    }

    #[test]
    fn test_cpu_arch_asset_basename() {
        assert_eq!(CpuArch::Amd64.asset_basename(), "Xray-linux-64");
        assert_eq!(CpuArch::Arm64.asset_basename(), "Xray-linux-arm64-v8a");
    }

    fn test_manager(install: &Path, root: &Path) -> WwpsCoreUpgradeManager {
        WwpsCoreUpgradeManager::new(WwpsCoreUpgradeConfig::new(
            "wwps-core",
            install.to_owned(),
            root.join("unused-backup"),
            root.join("temp"),
            CpuArch::Amd64,
        ))
        .unwrap()
    }

    #[test]
    fn wwps_core_upgrade_is_single_flight() {
        let first = WWPS_CORE_UPGRADE.try_enter().unwrap();
        let error = WWPS_CORE_UPGRADE.try_enter().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("wwps-core upgrade already in progress")
        );
        drop(first);
        WWPS_CORE_UPGRADE.try_enter().unwrap();
    }

    #[tokio::test]
    async fn wwps_core_health_is_bounded() {
        let error = bounded_health(|| async { anyhow::bail!("inactive") })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("after 10 attempts"));
    }

    #[tokio::test]
    async fn deploy_core_keeps_backup_beside_binary_until_health_commit() {
        let tmp = tempdir().unwrap();
        let install = tmp.path().join("install");
        let unpack = tmp.path().join("unpack");
        tokio::fs::create_dir_all(&install).await.unwrap();
        tokio::fs::create_dir_all(&unpack).await.unwrap();
        tokio::fs::write(install.join("wwps-core"), b"old")
            .await
            .unwrap();
        tokio::fs::write(unpack.join("xray"), b"new").await.unwrap();
        let manager = test_manager(&install, tmp.path());
        let published = manager.deploy_core(&unpack).await.unwrap();
        assert_eq!(published.backup.parent(), Some(install.as_path()));
        assert_eq!(tokio::fs::read(&published.backup).await.unwrap(), b"old");
        rollback_binary(&published).await.unwrap();
        assert_eq!(
            tokio::fs::read(install.join("wwps-core")).await.unwrap(),
            b"old"
        );
    }

    #[tokio::test]
    async fn failed_extract_removes_partial_unpack_directory() {
        let tmp = tempdir().unwrap();
        let install = tmp.path().join("install");
        tokio::fs::create_dir_all(&install).await.unwrap();
        let manager = test_manager(&install, tmp.path());
        tokio::fs::create_dir_all(&manager.config.temp_dir)
            .await
            .unwrap();
        let invalid = tmp.path().join("invalid.zip");
        tokio::fs::write(&invalid, b"not a zip").await.unwrap();
        assert!(manager.extract_archive(&invalid).await.is_err());
        assert_eq!(
            std::fs::read_dir(&manager.config.temp_dir).unwrap().count(),
            0
        );
    }
}
