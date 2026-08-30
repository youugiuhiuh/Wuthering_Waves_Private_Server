use crate::core::network::release_api::ReleaseResponse;

/// Parse the version token from `wwps-box version` output
/// (first line `sing-box version <ver>`).
pub fn parse_version_from_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("sing-box version ")?;
        let version = rest.trim();
        (!version.is_empty()).then(|| version.trim_start_matches('v').to_string())
    })
}

/// Build the sing-box release tarball download URL for a version and arch.
pub fn build_download_url(version: &str, arch: &str) -> String {
    format!(
        "https://github.com/SagerNet/sing-box/releases/download/v{}/sing-box-{}-linux-{}.tar.gz",
        version, version, arch
    )
}

/// Map GitHub release responses to their raw tag names, in order.
pub fn tag_names(releases: &[ReleaseResponse]) -> Vec<String> {
    releases.iter().map(|r| r.tag_name.clone()).collect()
}

use crate::common::{BotAdapter, MessageContent, TargetId};
use crate::core::network::release_api::{fetch_json_from_mirrors, fetch_prerelease};
use crate::core::paths::singbox;
use crate::core::singbox::installer::SingBoxInstaller;
use crate::core::utils::human_readable_size;
use anyhow::{Context, Result};
use rust_i18n::t;
use std::path::Path;
use tokio::fs;

const SINGBOX_RELEASE_OWNER: &str = "SagerNet";
const SINGBOX_RELEASE_REPO: &str = "sing-box";
const SINGBOX_RELEASE_API_BASE: &str = "https://api.github.com/repos";
const SINGBOX_UPGRADE_TEMP_DIR: &str = "/tmp/sing-box-upgrade";

#[derive(Debug, Clone)]
pub struct SingBoxReleaseInfo {
    pub tag_name: String,
    pub download_url: String,
    pub size: Option<u64>,
}

pub struct SingBoxUpgradeManager {
    client: reqwest::Client,
    github_token: Option<String>,
}

impl SingBoxUpgradeManager {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("构建 HTTP 客户端失败")?;
        let token = std::env::var("GITHUB_TOKEN").ok().filter(|v| !v.is_empty());
        Ok(Self {
            client,
            github_token: token,
        })
    }

    pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let path = format!(
            "{}/{}/releases?per_page={}",
            SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO, limit
        );
        let bases = vec![SINGBOX_RELEASE_API_BASE.to_string()];
        let releases: Vec<ReleaseResponse> = fetch_json_from_mirrors(
            &self.client,
            &bases,
            &path,
            self.github_token.as_deref(),
        )
        .await?;
        Ok(tag_names(&releases).into_iter().take(limit).collect())
    }

    pub async fn fetch_release(&self, tag: Option<&str>) -> Result<SingBoxReleaseInfo> {
        let bases = vec![SINGBOX_RELEASE_API_BASE.to_string()];
        let release: ReleaseResponse = if let Some(t) = tag {
            let path = format!(
                "{}/{}/releases/tags/{}",
                SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO, t
            );
            fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
                .await?
        } else {
            let path = format!(
                "{}/{}/releases?per_page=20",
                SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO
            );
            fetch_prerelease(&self.client, &bases, &path, self.github_token.as_deref()).await?
        };

        let version = release.tag_name.trim_start_matches('v');
        let arch = SingBoxInstaller::detect_arch()?;
        let download_url = build_download_url(version, arch);
        let tarball_name = format!("sing-box-{}-linux-{}.tar.gz", version, arch);
        let size = release
            .assets
            .iter()
            .find(|a| a.name == tarball_name)
            .and_then(|a| a.size);

        Ok(SingBoxReleaseInfo {
            tag_name: release.tag_name,
            download_url,
            size,
        })
    }

    pub async fn current_version() -> Option<String> {
        let output = tokio::process::Command::new(singbox::BIN)
            .arg("version")
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        parse_version_from_output(&text)
    }

    pub async fn run_upgrade(
        tag: Option<String>,
        adapter: &dyn BotAdapter,
        target: &TargetId,
    ) -> Result<()> {
        let status_msg_id = adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("menu.singbox_upgrade_checking").to_string(),
                    markup: None,
                },
            )
            .await?;

        let manager = SingBoxUpgradeManager::new()?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_fetching").to_string(),
                    markup: None,
                },
            )
            .await;

        let release = manager.fetch_release(tag.as_deref()).await?;

        let size_str = release
            .size
            .map(human_readable_size)
            .unwrap_or_else(|| t!("menu.singbox_upgrade_unknown_size").to_string());
        let info_text = t!(
            "menu.singbox_upgrade_download_info",
            "0" => release.tag_name.as_str(),
            "1" => size_str.as_str()
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

        fs::create_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await?;
        let archive_path = format!("{}/sing-box.tar.gz", SINGBOX_UPGRADE_TEMP_DIR);
        SingBoxInstaller::download_file(&release.download_url, &archive_path).await?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_extracting").to_string(),
                    markup: None,
                },
            )
            .await;
        SingBoxInstaller::extract_archive(&archive_path, SINGBOX_UPGRADE_TEMP_DIR).await?;

        let version = release.tag_name.trim_start_matches('v');
        let arch = SingBoxInstaller::detect_arch()?;
        let unpacked_bin = format!(
            "{}/sing-box-{}-linux-{}/sing-box",
            SINGBOX_UPGRADE_TEMP_DIR, version, arch
        );
        if !Path::new(&unpacked_bin).exists() {
            anyhow::bail!("未找到解压后的 sing-box 二进制: {}", unpacked_bin);
        }

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_replacing").to_string(),
                    markup: None,
                },
            )
            .await;
        fs::copy(&unpacked_bin, singbox::BIN)
            .await
            .context("复制 sing-box 二进制失败")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(singbox::BIN).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(singbox::BIN, perms).await?;
        }

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_restarting").to_string(),
                    markup: None,
                },
            )
            .await;
        SingBoxInstaller::restart_service().await?;

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_success", "0" => release.tag_name.as_str())
                        .to_string(),
                    markup: None,
                },
            )
            .await;

        let _ = fs::remove_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::network::release_api::ReleaseResponse;

    #[test]
    fn test_parse_version_from_output_typical() {
        let out = "sing-box version 1.14.0-rc.4\n\nEnvironment: go1.25.12 linux/amd64\n";
        assert_eq!(parse_version_from_output(out), Some("1.14.0-rc.4".to_string()));
    }

    #[test]
    fn test_parse_version_from_output_stable() {
        let out = "sing-box version 1.13.20\n";
        assert_eq!(parse_version_from_output(out), Some("1.13.20".to_string()));
    }

    #[test]
    fn test_parse_version_from_output_empty() {
        assert_eq!(parse_version_from_output(""), None);
        assert_eq!(parse_version_from_output("not a version line\n"), None);
    }

    #[test]
    fn test_build_download_url() {
        assert_eq!(
            build_download_url("1.14.0-rc.4", "amd64"),
            "https://github.com/SagerNet/sing-box/releases/download/v1.14.0-rc.4/sing-box-1.14.0-rc.4-linux-amd64.tar.gz"
        );
    }

    #[test]
    fn test_tag_names_maps_in_order() {
        let releases = vec![
            ReleaseResponse {
                tag_name: "v1.14.0-rc.4".to_string(),
                body: None,
                assets: vec![],
                prerelease: true,
            },
            ReleaseResponse {
                tag_name: "v1.13.20".to_string(),
                body: None,
                assets: vec![],
                prerelease: false,
            },
        ];
        assert_eq!(tag_names(&releases), vec!["v1.14.0-rc.4", "v1.13.20"]);
    }
}
