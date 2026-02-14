use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::task;
use tokio::time::sleep;

use crate::logic::cmd_async::run_cmd_status;
use crate::logic::utils::{format_download_progress, human_readable_size, should_report};

const DEFAULT_OWNER: &str = "youugiuhiuh";
const DEFAULT_REPO: &str = "Wuthering_Waves_Private_Server";
const DEFAULT_ASSET_NAME: &str = "tgbot";
const USER_AGENT_VALUE: &str = "tgbot-self-update";

pub const UPGRADE_FLAG_FILE: &str = "/etc/wwps/tgbot/upgrade.flag";

static SHA256_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)sha256[:\s]+([0-9a-f]{64})").expect("valid sha256 regex"));

pub struct UpgradeManager {
    client: reqwest::Client,
    owner: String,
    repo: String,
    asset_name: String,
    token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseArtifact {
    pub tag_name: String,
    pub asset_name: String,
    pub download_url: String,
    pub sha256: String,
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    body: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
    #[serde(default)]
    digest: Option<String>,
}

impl UpgradeManager {
    pub fn new() -> Result<Self> {
        let owner = env::var("TGBOT_RELEASE_OWNER").unwrap_or_else(|_| DEFAULT_OWNER.to_string());
        let repo = env::var("TGBOT_RELEASE_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
        let asset_name =
            env::var("TGBOT_RELEASE_ASSET").unwrap_or_else(|_| DEFAULT_ASSET_NAME.to_string());
        let token = env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("构建 HTTP 客户端失败")?;

        Ok(Self {
            client,
            owner,
            repo,
            asset_name,
            token,
        })
    }

    pub async fn run(self, bot: Bot, chat_id: ChatId) -> Result<()> {
        let mut progress_msg = bot
            .send_message(chat_id, "🔍 正在查询最新 Release...")
            .await?;

        let artifact = match self.fetch_latest_release().await {
            Ok(a) => a,
            Err(e) => {
                let _ = bot
                    .edit_message_text(
                        chat_id,
                        progress_msg.id,
                        format!("❌ 获取 Release 失败: {}", e),
                    )
                    .await;
                return Err(e);
            }
        };

        let summary = format!(
            "📦 最新版本: {tag}\n文件: {name}\n大小: {size}\nSHA256: {hash}",
            tag = artifact.tag_name,
            name = artifact.asset_name,
            size = artifact
                .size
                .map(human_readable_size)
                .unwrap_or_else(|| "未知".to_string()),
            hash = &artifact.sha256
        );

        progress_msg = bot
            .edit_message_text(
                chat_id,
                progress_msg.id,
                format!("{}\n\n准备开始下载...", summary),
            )
            .await?;

        let update_path = match self
            .download_with_progress(&artifact, &bot, chat_id, progress_msg.id)
            .await
        {
            Ok(path) => path,
            Err(e) => {
                let _ = bot
                    .edit_message_text(chat_id, progress_msg.id, format!("❌ 下载失败: {}", e))
                    .await;
                return Err(e);
            }
        };

        if let Err(e) = self
            .finalize_install(&artifact, &update_path, &bot, chat_id, progress_msg.id)
            .await
        {
            let _ = bot
                .edit_message_text(chat_id, progress_msg.id, format!("❌ 安装失败: {}", e))
                .await;
            let _ = fs::remove_file(&update_path).await;
            return Err(e);
        }

        Ok(())
    }

    async fn fetch_latest_release(&self) -> Result<ReleaseArtifact> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            self.owner, self.repo
        );

        let response = self
            .build_request(&url)
            .send()
            .await
            .context("请求 GitHub Release API 失败")?
            .error_for_status()
            .context("GitHub Release API 返回错误状态")?;

        let release: ReleaseResponse = response.json().await.context("解析 Release JSON 失败")?;
        let asset = self
            .select_asset(&release.assets)
            .ok_or_else(|| anyhow!("未找到匹配的 Release 产物 ({})", self.asset_name))?;

        let sha256 = if let Some(digest) = asset.digest.as_deref() {
            parse_digest(digest).ok_or_else(|| anyhow!("无法解析 digest 字段"))?
        } else if let Some(hash) = self
            .download_sha256_manifest(&release.assets, &asset.name)
            .await?
        {
            hash
        } else if let Some(body) = release.body.as_deref() {
            extract_sha256_from_body(body).ok_or_else(|| anyhow!("Release 中缺少 SHA256 信息"))?
        } else {
            anyhow::bail!("Release 中缺少 SHA256 信息");
        };

        Ok(ReleaseArtifact {
            tag_name: release.tag_name,
            asset_name: asset.name.clone(),
            download_url: asset.browser_download_url.clone(),
            sha256,
            size: asset.size,
        })
    }

    fn select_asset<'a>(&self, assets: &'a [ReleaseAsset]) -> Option<&'a ReleaseAsset> {
        assets
            .iter()
            .find(|a| a.name == self.asset_name)
            .or_else(|| assets.iter().find(|a| a.name.starts_with(&self.asset_name)))
            .or_else(|| assets.first())
    }

    async fn download_sha256_manifest(
        &self,
        assets: &[ReleaseAsset],
        target_asset: &str,
    ) -> Result<Option<String>> {
        let manifest = assets.iter().find(|asset| {
            asset.name.ends_with(".sha256")
                || asset.name.ends_with(".sha256.txt")
                || asset.name.ends_with(".sha256sum")
        });

        let Some(manifest_asset) = manifest else {
            return Ok(None);
        };

        let text = self
            .build_request(&manifest_asset.browser_download_url)
            .send()
            .await
            .context("下载 SHA256 校验文件失败")?
            .error_for_status()
            .context("SHA256 校验文件返回错误状态")?
            .text()
            .await
            .context("读取 SHA256 校验文件失败")?;

        Ok(parse_sha256_manifest(&text, target_asset))
    }

    async fn download_with_progress(
        &self,
        artifact: &ReleaseArtifact,
        bot: &Bot,
        chat_id: ChatId,
        progress_msg_id: MessageId,
    ) -> Result<PathBuf> {
        let response = self
            .build_request(&artifact.download_url)
            .send()
            .await
            .context("下载 Release 文件失败")?
            .error_for_status()
            .context("下载请求返回错误状态")?;

        let total_size = response.content_length();
        let mut stream = response.bytes_stream();

        let current_exe = std::env::current_exe().context("无法获取当前可执行文件路径")?;
        let update_path = current_exe.with_extension("update");
        let mut file = File::create(&update_path)
            .await
            .context("创建临时更新文件失败")?;
        let mut writer = tokio::io::BufWriter::new(&mut file);
        let mut hasher = Sha256::new();

        let mut downloaded: u64 = 0;
        let mut last_reported_pct = 0.0;
        let mut last_reported_size: u64 = 0;
        let mut last_report = Instant::now();
        let start = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取下载数据失败")?;
            writer.write_all(&chunk).await.context("写入更新文件失败")?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;

            if should_report(
                downloaded,
                total_size,
                &mut last_reported_pct,
                &mut last_reported_size,
                last_report,
            ) {
                last_report = Instant::now();
                let progress_text = format_download_progress(downloaded, total_size, start);
                let _ = bot
                    .edit_message_text(chat_id, progress_msg_id, progress_text)
                    .await;
            }
        }

        writer.flush().await.context("刷新更新文件失败")?;
        drop(writer);
        file.sync_all().await.context("同步更新文件到磁盘失败")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().await?.permissions();
            perms.set_mode(0o755);
            file.set_permissions(perms).await?;
        }

        let actual_sha256 = hex::encode(hasher.finalize());
        if actual_sha256 != artifact.sha256 {
            fs::remove_file(&update_path).await.ok();
            anyhow::bail!(
                "SHA256 校验失败，期望: {}, 实际: {}",
                artifact.sha256,
                actual_sha256
            );
        }

        let _ = bot
            .edit_message_text(chat_id, progress_msg_id, "✅ 下载完成，校验通过。")
            .await;

        Ok(update_path)
    }

    async fn finalize_install(
        &self,
        artifact: &ReleaseArtifact,
        update_path: &Path,
        bot: &Bot,
        chat_id: ChatId,
        progress_msg_id: MessageId,
    ) -> Result<()> {
        let _ = bot
            .edit_message_text(chat_id, progress_msg_id, "🔁 正在替换运行中的实例...")
            .await;

        let update_path_owned = update_path.to_path_buf();
        task::spawn_blocking(move || self_replace::self_replace(&update_path_owned))
            .await
            .context("等待替换任务失败")??;

        fs::remove_file(&update_path)
            .await
            .context("清理解压文件失败")
            .ok();

        self.write_upgrade_flag(&artifact.tag_name).await?;

        bot.send_message(
            chat_id,
            format!("✅ Bot 已更新到 {}，即将重启...", artifact.tag_name),
        )
        .await?;

        sleep(Duration::from_secs(2)).await;
        std::process::exit(0);
    }

    pub async fn write_upgrade_flag(&self, version: &str) -> Result<()> {
        if let Some(parent) = Path::new(UPGRADE_FLAG_FILE).parent() {
            fs::create_dir_all(parent)
                .await
                .context("创建升级标记目录失败")?;
        }
        fs::write(UPGRADE_FLAG_FILE, version)
            .await
            .context("写入升级标记文件失败")
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let builder = self.client.get(url).headers(headers);
        if let Some(token) = &self.token {
            builder.bearer_auth(token)
        } else {
            builder
        }
    }
}

pub mod wwps_core {
    use super::*;
    use std::fs as std_fs;
    use std::fs::{File as StdFile, OpenOptions};
    use std::sync::Arc;
    use teloxide::types::MessageId;
    use zip::ZipArchive;

    const WWPS_CORE_DEFAULT_OWNER: &str = "XTLS";
    const WWPS_CORE_DEFAULT_REPO: &str = "Xray-core";
    const WWPS_CORE_DEFAULT_SERVICE: &str = "wwps-core";
    const WWPS_CORE_DEFAULT_INSTALL_DIR: &str = "/etc/wwps/wwps-core";
    const WWPS_CORE_DEFAULT_TEMP_DIR: &str = "/tmp/wwps-core-upgrade";
    const WWPS_CORE_DEFAULT_BACKUP_PREFIX: &str = "wwps-core-backup";
    const WWPS_CORE_RELEASE_API: &str = "https://api.github.com/repos";

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

    #[derive(Debug, Clone)]
    pub struct WwpsCoreUpgradeConfig {
        pub owner: String,
        pub repo: String,
        pub service_name: String,
        pub install_dir: PathBuf,
        pub backup_dir: PathBuf,
        pub temp_dir: PathBuf,
        pub arch: CpuArch,
    }

    #[derive(Debug, Clone)]
    pub struct WwpsCoreReleaseInfo {
        pub tag_name: String,
        pub download_url: String,
        pub sha256: String,
        pub size: Option<u64>,
    }

    pub struct WwpsCoreUpgradeManager {
        config: Arc<WwpsCoreUpgradeConfig>,
        client: reqwest::Client,
        github_token: Option<String>,
    }

    impl WwpsCoreUpgradeConfig {
        pub fn new(
            owner: impl Into<String>,
            repo: impl Into<String>,
            service_name: impl Into<String>,
            install_dir: PathBuf,
            backup_dir: PathBuf,
            temp_dir: PathBuf,
            arch: CpuArch,
        ) -> Self {
            Self {
                owner: owner.into(),
                repo: repo.into(),
                service_name: service_name.into(),
                install_dir,
                backup_dir,
                temp_dir,
                arch,
            }
        }

        pub fn from_env() -> Result<Self> {
            let owner = env::var("WWPS_CORE_RELEASE_OWNER")
                .unwrap_or_else(|_| WWPS_CORE_DEFAULT_OWNER.to_string());
            let repo = env::var("WWPS_CORE_RELEASE_REPO")
                .unwrap_or_else(|_| WWPS_CORE_DEFAULT_REPO.to_string());
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
                owner,
                repo,
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

        #[allow(dead_code)]
        pub fn asset_basename(&self) -> &'static str {
            self.arch.asset_basename()
        }

        fn ensure_dir_writable(path: &Path) -> Result<()> {
            if !path.exists() {
                std_fs::create_dir_all(path)
                    .with_context(|| format!("创建目录失败: {}", path.display()))?;
            }

            let test_path = path.join(format!(".write-test-{}", std::process::id()));
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            opts.open(&test_path)
                .with_context(|| format!("目录不可写: {}", path.display()))?;
            std_fs::remove_file(&test_path).ok();
            Ok(())
        }
    }

    impl WwpsCoreUpgradeManager {
        pub fn new(config: WwpsCoreUpgradeConfig) -> Result<Self> {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .context("构建 HTTP 客户端失败")?;
            let token = env::var("GITHUB_TOKEN").ok().filter(|v| !v.is_empty());

            Ok(Self {
                config: Arc::new(config),
                client,
                github_token: token,
            })
        }

        pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>> {
            if limit == 0 {
                return Ok(vec![]);
            }

            let config = &self.config;
            let url = format!(
                "{}/{}/{}/releases?per_page={}",
                WWPS_CORE_RELEASE_API, config.owner, config.repo, limit
            );

            let response = self
                .build_request(&url)
                .send()
                .await
                .context("请求 wwps-core Release 列表失败")?
                .error_for_status()
                .context("wwps-core Release 列表返回错误状态")?;

            let releases: Vec<ReleaseResponse> = response
                .json()
                .await
                .context("解析 wwps-core Release 列表 JSON 失败")?;

            let tags = releases
                .into_iter()
                .map(|r| r.tag_name)
                .take(limit)
                .collect();

            Ok(tags)
        }

        pub async fn fetch_release(&self, tag: Option<&str>) -> Result<WwpsCoreReleaseInfo> {
            let config = &self.config;
            let url = if let Some(tag) = tag {
                format!(
                    "{}/{}/{}/releases/tags/{}",
                    WWPS_CORE_RELEASE_API, config.owner, config.repo, tag
                )
            } else {
                format!(
                    "{}/{}/{}/releases/latest",
                    WWPS_CORE_RELEASE_API, config.owner, config.repo
                )
            };

            let response = self
                .build_request(&url)
                .send()
                .await
                .context("请求 wwps-core Release 失败")?
                .error_for_status()
                .context("wwps-core Release API 返回错误状态")?;

            let release: ReleaseResponse = response
                .json()
                .await
                .context("解析 wwps-core Release JSON 失败")?;

            let asset_name = format!("{}.zip", config.arch.asset_basename());
            let asset = release
                .assets
                .iter()
                .find(|a| a.name == asset_name)
                .ok_or_else(|| anyhow!("未在 Release 中找到资产 {}", asset_name))?;

            let sha256 = if let Some(digest) = asset.digest.as_deref() {
                parse_digest(digest).ok_or_else(|| anyhow!("无法解析 digest 字段"))?
            } else if let Some(hash) = self
                .download_sha256_manifest(&release.assets, &asset.name)
                .await?
            {
                hash
            } else if let Some(body) = release.body.as_deref() {
                extract_sha256_from_body(body)
                    .ok_or_else(|| anyhow!("Release 中缺少 SHA256 信息"))?
            } else {
                anyhow::bail!("Release 中缺少 SHA256 信息");
            };

            Ok(WwpsCoreReleaseInfo {
                tag_name: release.tag_name,
                download_url: asset.browser_download_url.clone(),
                sha256,
                size: asset.size,
            })
        }

        pub async fn download_release(
            &self,
            release: &WwpsCoreReleaseInfo,
            bot: Option<&Bot>,
            chat_id: Option<ChatId>,
            msg_id: Option<MessageId>,
        ) -> Result<PathBuf> {
            let temp_file = self.config.temp_dir.join(format!(
                "wwps-core-{}-{}.zip",
                release.tag_name,
                Utc::now().timestamp()
            ));

            fs::create_dir_all(&self.config.temp_dir)
                .await
                .context("创建临时目录失败")?;

            let response = self
                .build_request(&release.download_url)
                .send()
                .await
                .context("下载 wwps-core Release 失败")?
                .error_for_status()
                .context("wwps-core Release 下载返回错误状态")?;

            let total_size = response.content_length();
            let mut stream = response.bytes_stream();
            let mut file = File::create(&temp_file)
                .await
                .context("创建 wwps-core 临时包失败")?;
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
                    .context("写入 wwps-core 临时包失败")?;
                downloaded += chunk.len() as u64;

                if let (Some(bot), Some(chat_id), Some(msg_id)) = (bot, chat_id, msg_id)
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
                    let _ = bot.edit_message_text(chat_id, msg_id, progress_text).await;
                }
            }

            writer.flush().await.context("刷新 wwps-core 临时包失败")?;
            drop(writer);
            file.sync_all().await.context("同步 wwps-core 包失败")?;

            let actual_hash = hex::encode(hasher.finalize());
            if actual_hash != release.sha256 {
                fs::remove_file(&temp_file).await.ok();
                anyhow::bail!(
                    "wwps-core 包 SHA256 校验失败，期望: {} 实际: {}",
                    release.sha256,
                    actual_hash
                );
            }

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

        pub async fn replace_core(&self, unpack_dir: &Path) -> Result<()> {
            // 注意：zip包里可能还是叫 xray
            let new_core = unpack_dir.join("xray");
            if !new_core.exists() {
                anyhow::bail!("解压目录中未找到 xray 可执行文件");
            }

            let target_core = self.config.install_dir.join("wwps-core.new");
            fs::copy(&new_core, &target_core)
                .await
                .context("拷贝新核心失败")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = fs::metadata(&target_core).await?;
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target_core, perms)
                    .await
                    .context("设置 wwps-core 可执行权限失败")?;
            }

            let final_target = self.config.install_dir.join("wwps-core");
            fs::rename(&target_core, &final_target)
                .await
                .context("替换 wwps-core 核心失败")?;

            Ok(())
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
            let status =
                run_cmd_status("systemctl", &["is-active", &unit], Duration::from_secs(15))
                    .await
                    .context("执行 systemctl is-active 失败")?;

            if status.success() {
                Ok(())
            } else {
                anyhow::bail!("{} 未在运行", unit);
            }
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

        pub async fn run_upgrade(tag: Option<String>, bot: Bot, chat_id: ChatId) -> Result<()> {
            let mut status_msg = bot
                .send_message(chat_id, "🛰️ 正在检查 wwps-core 环境...")
                .await?;

            let config = WwpsCoreUpgradeConfig::from_env()?;
            config.validate()?;
            let manager = WwpsCoreUpgradeManager::new(config)?;

            if let Ok(updated) = bot
                .edit_message_text(chat_id, status_msg.id, "📦 正在获取 wwps-core 版本信息...")
                .await
            {
                status_msg = updated;
            }

            let release = manager.fetch_release(tag.as_deref()).await?;

            let info_text = format!(
                "📦 准备下载 wwps-core {}
文件大小: {}
SHA256: {}",
                release.tag_name,
                release
                    .size
                    .map(human_readable_size)
                    .unwrap_or_else(|| "未知".to_string()),
                release.sha256
            );
            if let Ok(updated) = bot
                .edit_message_text(chat_id, status_msg.id, info_text)
                .await
            {
                status_msg = updated;
            }

            let archive_path = manager
                .download_release(&release, Some(&bot), Some(chat_id), Some(status_msg.id))
                .await?;

            if let Ok(updated) = bot
                .edit_message_text(chat_id, status_msg.id, "🗜️ 正在解压核心...")
                .await
            {
                status_msg = updated;
            }
            let unpack_dir = manager.extract_archive(&archive_path).await?;

            if let Ok(updated) = bot
                .edit_message_text(chat_id, status_msg.id, "💾 正在备份当前核心...")
                .await
            {
                status_msg = updated;
            }
            let backup_path = manager.backup_current_core().await?;

            if let Ok(updated) = bot
                .edit_message_text(chat_id, status_msg.id, "♻️ 正在替换核心...")
                .await
            {
                status_msg = updated;
            }
            manager.replace_core(&unpack_dir).await?;

            let _ = bot
                .edit_message_text(chat_id, status_msg.id, "🔁 正在重启 wwps-core 服务...")
                .await;

            manager.restart_service().await?;
            manager.verify_service_active().await?;

            manager
                .cleanup_paths(&[archive_path.clone(), unpack_dir.clone()])
                .await;

            bot.send_message(
                chat_id,
                format!(
                    "✅ wwps-core 已更新至 {}！\n备份目录: {}",
                    release.tag_name,
                    backup_path.display()
                ),
            )
            .await?;

            Ok(())
        }

        async fn download_sha256_manifest(
            &self,
            assets: &[ReleaseAsset],
            target_asset: &str,
        ) -> Result<Option<String>> {
            let manifest = assets.iter().find(|asset| {
                asset.name.ends_with(".sha256")
                    || asset.name.ends_with(".sha256.txt")
                    || asset.name.ends_with(".sha256sum")
            });

            let Some(manifest_asset) = manifest else {
                return Ok(None);
            };

            let text = self
                .build_request(&manifest_asset.browser_download_url)
                .send()
                .await
                .context("下载 SHA256 清单失败")?
                .error_for_status()
                .context("SHA256 清单返回错误状态")?
                .text()
                .await
                .context("读取 SHA256 清单失败")?;

            Ok(parse_sha256_manifest(&text, target_asset))
        }

        fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
            let mut headers = HeaderMap::new();
            headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
            headers.insert(
                ACCEPT,
                HeaderValue::from_static("application/vnd.github+json"),
            );
            let builder = self.client.get(url).headers(headers);
            if let Some(token) = &self.github_token {
                builder.bearer_auth(token)
            } else {
                builder
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use tempfile::tempdir;

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
            std_fs::create_dir_all(&install_dir).unwrap();
            std_fs::write(install_dir.join("wwps-core"), b"binary").unwrap();
            let backup_dir = tmp.path().join("backup");
            let temp_dir = tmp.path().join("temp");

            let config = WwpsCoreUpgradeConfig::new(
                "owner",
                "repo",
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
            std_fs::create_dir_all(&install_dir).unwrap();
            let backup_dir = tmp.path().join("backup");
            let temp_dir = tmp.path().join("temp");

            let config = WwpsCoreUpgradeConfig::new(
                "owner",
                "repo",
                "wwps-core",
                install_dir,
                backup_dir,
                temp_dir,
                CpuArch::Amd64,
            );

            assert!(config.validate().is_err());
        }
    }
}

fn parse_digest(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    lower
        .strip_prefix("sha256:")
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() == 64)
}

fn parse_sha256_manifest(manifest: &str, target_asset: &str) -> Option<String> {
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

fn extract_sha256_from_body(body: &str) -> Option<String> {
    SHA256_LINE_RE
        .captures(body)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::utils::PROGRESS_SIZE_STEP;
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
    fn test_parse_sha256_manifest() {
        let manifest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  tgbot\n";
        let result = parse_sha256_manifest(manifest, "tgbot");
        assert_eq!(
            result,
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
        );
        assert!(parse_sha256_manifest("", "tgbot").is_none());
    }

    #[test]
    fn test_extract_sha256_from_body() {
        let body = "Release notes\nSHA256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = extract_sha256_from_body(body);
        assert!(result.is_some());
        assert!(extract_sha256_from_body("no hash here").is_none());
    }

    #[test]
    fn test_should_report_on_percent_and_size() {
        let mut last_pct = 0.0;
        let mut last_size = 0;
        let instant = Instant::now();
        assert!(should_report(
            10,
            Some(100),
            &mut last_pct,
            &mut last_size,
            instant
        ));
        // Already reported, less than thresholds
        assert!(!should_report(
            12,
            Some(100),
            &mut last_pct,
            &mut last_size,
            Instant::now()
        ));
        // Large jump in bytes should trigger even without percent change
        assert!(should_report(
            last_size + PROGRESS_SIZE_STEP + 1,
            None,
            &mut last_pct,
            &mut last_size,
            Instant::now()
        ));
    }

    #[test]
    fn test_format_download_progress_with_total_and_unknown() {
        let start = Instant::now() - Duration::from_secs(1);
        let with_total = format_download_progress(5 * 1024 * 1024, Some(10 * 1024 * 1024), start);
        assert!(with_total.contains("50.0%"));
        let unknown_total = format_download_progress(1024, None, start);
        assert!(unknown_total.contains("总大小未知"));
    }

    #[test]
    fn test_human_readable_size_scaling() {
        assert_eq!(human_readable_size(512), "512 B");
        assert_eq!(human_readable_size(1024), "1.00 KB");
        assert_eq!(human_readable_size(1024 * 1024), "1.00 MB");
    }
}
