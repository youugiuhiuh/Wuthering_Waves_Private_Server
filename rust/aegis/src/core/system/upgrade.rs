use crate::adapters::common::{BotAdapter, MessageContent, MessageId as AegisMsgId, TargetId};
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use obfstr::obfstr;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::task;
use tokio::time::sleep;

use crate::core::network::release_api::{
    ReleaseAsset, ReleaseResponse, extract_sha256_from_body, fetch_json_from_mirrors, parse_digest,
    parse_sha256_manifest,
};
use crate::core::utils::{format_download_progress, human_readable_size, should_report};

const DEFAULT_RELEASE_REPOSITORIES: &[(&str, &str)] = &[
    ("NicholasDewar", "Wuthering_Waves_Private_Server"),
    ("youugiuhiuh", "Wuthering_Waves_Private_Server"),
];
const DEFAULT_ASSET_NAME: &str = "aegis";
const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

/// Release API 根地址列表（支持 GitHub / Codeberg / Gitea 等兼容 API），按顺序尝试
fn aegis_release_api_bases() -> Vec<String> {
    if let Ok(s) = env::var("AEGIS_RELEASE_MIRRORS") {
        let bases: Vec<String> = s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        if !bases.is_empty() {
            return bases;
        }
    }
    vec![
        "https://api.github.com".to_string(),
        "https://codeberg.org/api/v1".to_string(),
        "https://gitea.com/api/v1".to_string(),
    ]
}

pub use crate::core::paths::maintenance::UPGRADE_FLAG_FILE;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseRepo {
    owner: String,
    repo: String,
}

impl ReleaseRepo {
    fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    fn display_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

fn parse_release_repo(input: &str) -> Option<ReleaseRepo> {
    let trimmed = input.trim().trim_matches('/');
    let (owner, repo) = trimmed.split_once('/')?;
    let owner = owner.trim();
    let repo = repo.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ReleaseRepo::new(owner, repo))
}

fn configured_release_repositories() -> Vec<ReleaseRepo> {
    if let Ok(value) = env::var("AEGIS_RELEASE_REPOSITORIES") {
        let repos: Vec<ReleaseRepo> = value.split(',').filter_map(parse_release_repo).collect();
        if !repos.is_empty() {
            return repos;
        }
    }

    if let Ok(value) = env::var("AEGIS_RELEASE_REPOSITORY")
        && let Some(repo) = parse_release_repo(&value)
    {
        return vec![repo];
    }

    match (
        env::var("AEGIS_RELEASE_OWNER"),
        env::var("AEGIS_RELEASE_REPO"),
    ) {
        (Ok(owner), Ok(repo)) if !owner.trim().is_empty() && !repo.trim().is_empty() => {
            return vec![ReleaseRepo::new(owner.trim(), repo.trim())];
        }
        _ => {}
    }

    DEFAULT_RELEASE_REPOSITORIES
        .iter()
        .map(|(owner, repo)| ReleaseRepo::new(*owner, *repo))
        .collect()
}

pub struct UpgradeManager {
    client: reqwest::Client,
    repositories: Vec<ReleaseRepo>,
    asset_name: String,
    token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseArtifact {
    pub repository: String,
    pub tag_name: String,
    pub asset_name: String,
    pub download_url: String,
    pub sha256: String,
    pub size: Option<u64>,
}

impl UpgradeManager {
    pub fn new() -> Result<Self> {
        let repositories = configured_release_repositories();
        let asset_name =
            env::var("AEGIS_RELEASE_ASSET").unwrap_or_else(|_| DEFAULT_ASSET_NAME.to_string());
        let token = env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("构建 HTTP 客户端失败")?;

        Ok(Self {
            client,
            repositories,
            asset_name,
            token,
        })
    }

    pub async fn run(self, adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
        let progress_msg_id = adapter
            .send_message(
                target,
                MessageContent {
                    text: format!("{}", obfstr!("🔍 正在查询最新 Release...")),
                    markup: None,
                },
            )
            .await?;

        let artifact = match self.fetch_latest_release().await {
            Ok(a) => a,
            Err(e) => {
                let _ = adapter
                    .edit_message(
                        target,
                        &progress_msg_id,
                        MessageContent {
                            text: format!("❌ 获取 Release 失败: {}", e),
                            markup: None,
                        },
                    )
                    .await;
                return Err(e);
            }
        };

        let summary = format!(
            "📦 仓库: {repo}\n最新版本: {tag}\n文件: {name}\n大小: {size}\nSHA256: {hash}",
            repo = artifact.repository,
            tag = artifact.tag_name,
            name = artifact.asset_name,
            size = artifact
                .size
                .map(human_readable_size)
                .unwrap_or_else(|| "未知".to_string()),
            hash = &artifact.sha256
        );

        let _ = adapter
            .edit_message(
                target,
                &progress_msg_id,
                MessageContent {
                    text: format!("{}\n\n准备开始下载...", summary),
                    markup: None,
                },
            )
            .await;

        let update_path = match self
            .download_with_progress(&artifact, adapter, target, &progress_msg_id)
            .await
        {
            Ok(path) => path,
            Err(e) => {
                let _ = adapter
                    .edit_message(
                        target,
                        &progress_msg_id,
                        MessageContent {
                            text: format!("❌ 下载失败: {}", e),
                            markup: None,
                        },
                    )
                    .await;
                return Err(e);
            }
        };

        if let Err(e) = self
            .finalize_install(&artifact, &update_path, adapter, target, &progress_msg_id)
            .await
        {
            let _ = adapter
                .edit_message(
                    target,
                    &progress_msg_id,
                    MessageContent {
                        text: format!("❌ 安装失败: {}", e),
                        markup: None,
                    },
                )
                .await;
            let _ = fs::remove_file(&update_path).await;
            return Err(e);
        }

        Ok(())
    }

    async fn fetch_latest_release(&self) -> Result<ReleaseArtifact> {
        let bases = aegis_release_api_bases();
        let mut errors = Vec::new();

        for repository in &self.repositories {
            let api_path = format!(
                "repos/{}/{}/releases/latest",
                repository.owner, repository.repo
            );

            match self
                .fetch_latest_release_from_repo(repository, &bases, &api_path)
                .await
            {
                Ok(artifact) => return Ok(artifact),
                Err(err) => errors.push(format!("{}: {}", repository.display_name(), err)),
            }
        }

        anyhow::bail!("获取 Release 失败，已尝试: {}", errors.join(" | "))
    }

    async fn fetch_latest_release_from_repo(
        &self,
        repository: &ReleaseRepo,
        bases: &[String],
        api_path: &str,
    ) -> Result<ReleaseArtifact> {
        let release: ReleaseResponse =
            fetch_json_from_mirrors(&self.client, bases, api_path, self.token.as_deref()).await?;

        if release.tag_name.is_empty() {
            anyhow::bail!("Release 缺少 tag_name");
        }

        let asset = self
            .select_asset(&release.assets)
            .ok_or_else(|| anyhow!("未找到匹配的 Release 产物 ({})", self.asset_name))?;

        let download_url = asset.download_url().to_string();
        if download_url.is_empty() {
            anyhow::bail!("Release 产物无下载地址");
        }

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
            repository: repository.display_name(),
            tag_name: release.tag_name,
            asset_name: asset.name.clone(),
            download_url,
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

        let manifest_url = manifest_asset.download_url();
        if manifest_url.is_empty() {
            return Ok(None);
        }

        let text = self
            .build_request(manifest_url)
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
        adapter: &dyn BotAdapter,
        target: &TargetId,
        progress_msg_id: &AegisMsgId,
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
                let _ = adapter
                    .edit_message(
                        target,
                        progress_msg_id,
                        MessageContent {
                            text: progress_text,
                            markup: None,
                        },
                    )
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

        let _ = adapter
            .edit_message(
                target,
                progress_msg_id,
                MessageContent {
                    text: "✅ 下载完成，校验通过。".to_string(),
                    markup: None,
                },
            )
            .await;

        Ok(update_path)
    }

    async fn finalize_install(
        &self,
        artifact: &ReleaseArtifact,
        update_path: &Path,
        adapter: &dyn BotAdapter,
        target: &TargetId,
        progress_msg_id: &AegisMsgId,
    ) -> Result<()> {
        let _ = adapter
            .edit_message(
                target,
                progress_msg_id,
                MessageContent {
                    text: "🔁 正在替换运行中的实例...".to_string(),
                    markup: None,
                },
            )
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

        adapter
            .send_message(
                target,
                MessageContent {
                    text: format!("✅ Bot 已更新到 {}，即将重启...", artifact.tag_name),
                    markup: None,
                },
            )
            .await?;

        sleep(Duration::from_secs(2)).await;
        std::process::exit(0);
    }

    pub async fn write_upgrade_flag(&self, version: &str) -> Result<()> {
        let flag_path = obfstr!("/etc/wwps/aegis/upgrade.flag").to_string();
        if let Some(parent) = Path::new(&flag_path).parent() {
            fs::create_dir_all(parent)
                .await
                .context(obfstr!("创建升级标记目录失败").to_string())?;
        }
        fs::write(&flag_path, version)
            .await
            .context(obfstr!("写入升级标记文件失败").to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::utils::PROGRESS_SIZE_STEP;
    use std::time::Duration;

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

    #[test]
    fn test_parse_release_repo() {
        assert_eq!(
            parse_release_repo("youugiuhiuh/Wuthering_Waves_Private_Server_source_code"),
            Some(ReleaseRepo::new(
                "youugiuhiuh",
                "Wuthering_Waves_Private_Server_source_code"
            ))
        );
        assert!(parse_release_repo("invalid").is_none());
        assert!(parse_release_repo("/missing-owner").is_none());
    }
}
