use crate::adapters::common::{BotAdapter, MessageContent, MessageId as AegisMsgId, TargetId};
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use rust_i18n::t;
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

use crate::core::crypto::minisign::{self, MINISIGN_PUBLIC_KEYS};
use crate::core::network::release_api::{
    ReleaseResponse, build_asset_request, fetch_github_json, find_named_asset, github_api_client,
    github_asset_client, parse_digest,
};
use crate::core::system::upgrade_observer::{cancel_observer, prepare_observer};
use crate::core::system::upgrade_transaction::{
    SingleFlight, publish_binary, stage_binary, stage_path,
};
use crate::core::utils::{format_download_progress, human_readable_size, should_report};

static AEGIS_UPGRADE: SingleFlight = SingleFlight::new("aegis");

const AEGIS_RELEASE_OWNER: &str = "youugiuhiuh";
const AEGIS_RELEASE_REPO: &str = "Wuthering_Waves_Private_Server";
const AEGIS_RELEASE_ASSET: &str = "aegis";
const GITHUB_API_PATH: &str = "repos/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest";

pub struct UpgradeManager {
    client: reqwest::Client,
    asset_client: reqwest::Client,
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
    pub minisig: Vec<u8>,
}

impl UpgradeManager {
    pub fn new() -> Result<Self> {
        let token = env::var("GITHUB_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        Ok(Self {
            client: github_api_client(Duration::from_secs(60))?,
            asset_client: github_asset_client(Duration::from_secs(60))?,
            token,
        })
    }

    pub async fn run(self, adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
        let _flight = AEGIS_UPGRADE.try_enter()?;
        let progress_msg_id = adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("upgrade.bot_querying").to_string(),
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
                            text: t!("upgrade.bot_fetch_fail", "0" => e.to_string().as_str())
                                .to_string(),
                            markup: None,
                        },
                    )
                    .await;
                return Err(e);
            }
        };

        let size_str = artifact
            .size
            .map(human_readable_size)
            .unwrap_or_else(|| t!("upgrade.bot_unknown_size").to_string());
        let summary = t!(
            "upgrade.bot_summary",
            "0" => artifact.repository.as_str(),
            "1" => artifact.tag_name.as_str(),
            "2" => artifact.asset_name.as_str(),
            "3" => size_str.as_str(),
            "4" => artifact.sha256.as_str(),
        )
        .to_string();

        let _ = adapter
            .edit_message(
                target,
                &progress_msg_id,
                MessageContent {
                    text: t!("upgrade.bot_preparing", "0" => summary.as_str()).to_string(),
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
                            text: t!("upgrade.bot_download_fail", "0" => e.to_string().as_str())
                                .to_string(),
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
                        text: t!("upgrade.bot_install_fail", "0" => e.to_string().as_str())
                            .to_string(),
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
        let release = fetch_github_json::<ReleaseResponse>(
            &self.client,
            GITHUB_API_PATH,
            self.token.as_deref(),
        )
        .await?;

        if release.tag_name.is_empty() {
            anyhow::bail!("Release 缺少 tag_name");
        }

        let asset = find_named_asset(&release.assets, AEGIS_RELEASE_ASSET)
            .ok_or_else(|| anyhow!("未找到 Release 产物 ({})", AEGIS_RELEASE_ASSET))?;

        let download_url = asset.download_url().to_string();
        if download_url.is_empty() {
            anyhow::bail!("Release 产物无下载地址");
        }

        let sha256 = asset
            .digest
            .as_deref()
            .and_then(parse_digest)
            .ok_or_else(|| anyhow!("Release 产物缺少 SHA256 digest"))?;

        let minisig_asset =
            find_named_asset(&release.assets, &format!("{AEGIS_RELEASE_ASSET}.minisig"))
                .ok_or_else(|| anyhow!("未找到 Minisign 签名 ({}.minisig)", AEGIS_RELEASE_ASSET))?;

        let minisig_url = minisig_asset.download_url();
        if minisig_url.is_empty() {
            anyhow::bail!("Minisign 签名产物无下载地址");
        }

        let minisig = build_asset_request(&self.asset_client, minisig_url)?
            .send()
            .await
            .context("下载 Minisign 签名失败")?
            .error_for_status()
            .context("Minisign 签名下载返回错误状态")?
            .bytes()
            .await
            .context("读取 Minisign 签名失败")?
            .to_vec();

        Ok(ReleaseArtifact {
            repository: format!("{}/{}", AEGIS_RELEASE_OWNER, AEGIS_RELEASE_REPO),
            tag_name: release.tag_name,
            asset_name: AEGIS_RELEASE_ASSET.to_string(),
            download_url,
            sha256,
            size: asset.size,
            minisig,
        })
    }

    async fn verify_downloaded_update(
        &self,
        path: &Path,
        artifact: &ReleaseArtifact,
    ) -> Result<()> {
        let result = async {
            let data = fs::read(path).await.context("读取 Aegis 更新文件失败")?;
            let actual_sha256 = hex::encode(Sha256::digest(&data));
            if actual_sha256 != artifact.sha256 {
                anyhow::bail!("Aegis SHA256 校验失败");
            }
            let signature =
                std::str::from_utf8(&artifact.minisig).context("Aegis Minisign 不是有效 UTF-8")?;
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

    async fn download_with_progress(
        &self,
        artifact: &ReleaseArtifact,
        adapter: &dyn BotAdapter,
        target: &TargetId,
        progress_msg_id: &AegisMsgId,
    ) -> Result<PathBuf> {
        let response = build_asset_request(&self.asset_client, &artifact.download_url)?
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
        let mut downloaded: u64 = 0;
        let mut last_reported_pct = 0.0;
        let mut last_reported_size: u64 = 0;
        let mut last_report = Instant::now();
        let start = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取下载数据失败")?;
            writer.write_all(&chunk).await.context("写入更新文件失败")?;
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

        self.verify_downloaded_update(&update_path, artifact)
            .await?;

        let _ = adapter
            .edit_message(
                target,
                progress_msg_id,
                MessageContent {
                    text: t!("upgrade.bot_minisign_ok").to_string(),
                    markup: None,
                },
            )
            .await;

        let _ = adapter
            .edit_message(
                target,
                progress_msg_id,
                MessageContent {
                    text: t!("upgrade.bot_download_done").to_string(),
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
                    text: t!("upgrade.bot_replacing").to_string(),
                    markup: None,
                },
            )
            .await;

        let destination = std::env::current_exe().context("无法获取当前可执行文件路径")?;
        let staged = stage_binary(update_path, &destination).await?;
        if let Err(error) = fs::remove_file(update_path).await {
            let _ = fs::remove_file(&staged).await;
            return Err(error).context("清理下载更新文件失败");
        }
        let record = match prepare_observer(&artifact.tag_name, target, &destination).await {
            Ok(record) => record,
            Err(error) => {
                let _ = fs::remove_file(&staged).await;
                return Err(error);
            }
        };
        if let Err(error) = publish_binary(&staged, &destination).await {
            cancel_observer(&record).await;
            let _ = fs::remove_file(stage_path(&destination)?).await;
            return Err(error);
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
        std::process::exit(0);
    }
}

fn validate_trusted_comment(comment: &str, expected_tag: &str, expected_asset: &str) -> Result<()> {
    let (tag, asset) = minisign::parse_trusted_comment(comment)?;
    if tag != expected_tag || asset != expected_asset {
        anyhow::bail!("Minisign trusted comment 与 Release 不匹配");
    }
    Ok(())
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
        assert!(!should_report(
            12,
            Some(100),
            &mut last_pct,
            &mut last_size,
            Instant::now()
        ));
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
    fn test_new_constructor_succeeds() {
        let _manager = UpgradeManager::new().expect("constructor should succeed");
    }

    #[tokio::test]
    async fn verify_downloaded_update_rejects_corrupted_signature() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("aegis.update");
        let data = b"downloaded binary";
        tokio::fs::write(&path, data).await.unwrap();
        let artifact = ReleaseArtifact {
            repository: String::from("test/repo"),
            tag_name: String::from("v1.0.0"),
            asset_name: String::from("aegis"),
            download_url: String::from("https://github.com/o/r/releases/download/v1.0.0/aegis"),
            sha256: hex::encode(Sha256::digest(data)),
            size: Some(data.len() as u64),
            minisig: b"corrupted signature data".to_vec(),
        };
        let manager = UpgradeManager::new().unwrap();
        assert!(
            manager
                .verify_downloaded_update(&path, &artifact)
                .await
                .is_err()
        );
        assert!(
            !path.exists(),
            "temporary update file should be removed on failure"
        );
    }

    #[test]
    fn aegis_release_identity_is_fixed() {
        assert_eq!(AEGIS_RELEASE_OWNER, "youugiuhiuh");
        assert_eq!(AEGIS_RELEASE_REPO, "Wuthering_Waves_Private_Server");
        assert_eq!(AEGIS_RELEASE_ASSET, "aegis");
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
        assert!(
            manager
                .verify_downloaded_update(&path, &artifact)
                .await
                .is_err()
        );
        assert!(
            !path.exists(),
            "temporary update file should be removed on failure"
        );
    }

    #[test]
    fn aegis_upgrade_is_single_flight() {
        let first = AEGIS_UPGRADE.try_enter().unwrap();
        let error = AEGIS_UPGRADE.try_enter().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("aegis upgrade already in progress")
        );
        drop(first);
        AEGIS_UPGRADE.try_enter().unwrap();
    }

    #[test]
    fn trusted_comment_requires_exact_tag_and_asset() {
        validate_trusted_comment("v3.4.4:aegis", "v3.4.4", "aegis").unwrap();
        assert!(validate_trusted_comment("release-v3.4.4:aegis", "v3.4.4", "aegis").is_err());
        assert!(validate_trusted_comment("v3.4.4:aegis:extra", "v3.4.4", "aegis").is_err());
        assert!(validate_trusted_comment("v3.4.4:other", "v3.4.4", "aegis").is_err());
    }
}
