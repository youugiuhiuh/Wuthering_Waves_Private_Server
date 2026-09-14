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
use crate::core::network::release_api::{
    ReleaseAsset, fetch_json_from_mirrors, fetch_prerelease, parse_digest,
};
use crate::core::paths::singbox;
use crate::core::singbox::installer::SingBoxInstaller;
use crate::core::utils::human_readable_size;
use anyhow::{Context, Result, anyhow};
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
    /// 上游 release API 提供的资产 SHA256（hex，无前缀）。缺失即视为错误。
    pub sha256: String,
}

/// 构造 GitHub attestation 查询路径。返回空串表示 hex 无效，调用方不得发请求。
///
/// API 形态（实测）：已知 digest → 200；未知 digest → 404。
fn attestation_path(sha256_hex: &str) -> String {
    let ok = sha256_hex.len() == 64 && sha256_hex.bytes().all(|b| b.is_ascii_hexdigit());
    if !ok {
        return String::new();
    }
    format!(
        "/repos/{}/{}/attestations/sha256:{}",
        SINGBOX_RELEASE_OWNER,
        SINGBOX_RELEASE_REPO,
        sha256_hex.to_ascii_lowercase()
    )
}

/// 从资产列表中取指定资产名的 SHA256（hex）。缺失或格式非法 → None。
fn find_asset_sha256(assets: &[ReleaseAsset], asset_name: &str) -> Option<String> {
    assets
        .iter()
        .find(|a| a.name == asset_name)
        .and_then(|a| parse_digest(a.digest.as_deref().unwrap_or("")))
}

/// 计算文件 sha256 并与期望值比对（大小写不敏感）。
async fn verify_sha256_file(path: &str, expected_hex: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let data = fs::read(path).await.context("读取下载文件失败")?;
    let mut h = Sha256::new();
    h.update(&data);
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(expected_hex) {
        anyhow::bail!(
            "sing-box 校验失败: SHA256 期望 {}, 实际 {}",
            expected_hex,
            got
        );
    }
    Ok(())
}

/// 由 sha256 构造 attestation 查询的**完整** URL；hex 无效时返回空串。
///
/// ⚠️ 切勿改回 `format!("{}{}", SINGBOX_RELEASE_API_BASE, path)`：
/// `SINGBOX_RELEASE_API_BASE` 已经以 `/repos` 结尾，而 `attestation_path` 的返回值
/// 又以 `/repos` 开头，两者直接拼接会得到 `/repos/repos/...`，实测该 URL 恒为 404，
/// 使 `has_attestation_for` 永远返回 false，进而让首装/升级 100% 失败。
/// 故此处固定用主机名拼接（常量本身不能改：`fetch_recent_tags` / `fetch_release`
/// 依赖它拼 `.../repos/SagerNet/sing-box/releases...`）。
fn attestation_url(sha256_hex: &str) -> String {
    let path = attestation_path(sha256_hex);
    if path.is_empty() {
        return String::new();
    }
    format!("https://api.github.com{}", path)
}

/// 纯决策函数：HTTP 状态码 + 已解析响应体 → 该 digest 是否有 attestation。
///
/// - 404            → `Ok(false)`（该 digest 无记录）
/// - 其他非 2xx     → `Err`（fail-closed，不得当作无记录放过）
/// - 2xx 但无 body  → `Err`（无法判定）
/// - 2xx + body     → `Ok(attestations 数组非空)`（字段缺失或为空 → false）
fn attestation_decision(status: u16, body: Option<&serde_json::Value>) -> Result<bool> {
    if status == 404 {
        return Ok(false);
    }
    if !(200..300).contains(&status) {
        anyhow::bail!("attestation 查询返回 HTTP {}", status);
    }
    let body = body.ok_or_else(|| anyhow!("attestation 查询返回 {} 但响应体缺失", status))?;
    let n = body
        .get("attestations")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(n > 0)
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

    /// 调 GitHub attestation API 确认该 digest 确有 attestation 记录。
    /// 200 且含 ≥1 条 attestation → true；404 → false；其他状态码 → Err。
    ///
    /// 信任锚：attestation 由 GitHub 自己签发（SAN `dotcom.releases.github.com`），
    /// 且只能针对真实上传的资产生成 —— 故网络中间人无法为一个被篡改的文件
    /// 造出对应 digest 的 attestation 记录。
    ///
    /// 注：本函数不验证 sigstore 签名链本身（方案 A 的已知取舍）。
    async fn has_attestation_for(&self, sha256_hex: &str) -> Result<bool> {
        let url = attestation_url(sha256_hex);
        if url.is_empty() {
            anyhow::bail!("无效的 sha256: {}", sha256_hex);
        }
        let mut req = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json");
        if let Some(t) = self.github_token.as_deref() {
            req = req.header("Authorization", format!("Bearer {}", t));
        }
        let resp = req.send().await.context("查询 attestation 失败")?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            // 404 → 无记录；其他错误状态 → fail-closed（attestation_decision 内部裁决）
            return attestation_decision(status, None);
        }
        let json: serde_json::Value = resp.json().await.context("解析 attestation 响应失败")?;
        attestation_decision(status, Some(&json))
    }

    /// 端到端完整性校验（方案 A）：先 SHA256，再确认 GitHub 存有该 digest 的 attestation。
    ///
    /// 两条下载入径（首装 `installer.rs` 与升级 `upgrade.rs`）共用此函数。
    pub(crate) async fn verify_download(&self, path: &str, sha256_hex: &str) -> Result<()> {
        verify_sha256_file(path, sha256_hex).await?;
        if !self.has_attestation_for(sha256_hex).await? {
            anyhow::bail!(
                "sing-box 完整性校验失败: {} 无 GitHub attestation 记录（可能被替换）",
                sha256_hex
            );
        }
        Ok(())
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
        let releases: Vec<ReleaseResponse> =
            fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
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

        // SHA256 必需：上游不提供 minisign，digest 是唯一的完整性元数据。
        // 缺失则直接失败 —— 不得因为上游没给 digest 就跳过校验。
        let sha256 = find_asset_sha256(&release.assets, &tarball_name).ok_or_else(|| {
            anyhow!(
                "Release {} 的资产 {} 缺少 SHA256 digest",
                release.tag_name,
                tarball_name
            )
        })?;

        Ok(SingBoxReleaseInfo {
            tag_name: release.tag_name,
            download_url,
            size,
            sha256,
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

        // 先清理上次可能残留的暂存目录，避免旧文件污染新解压结果，也避免 /tmp(tmpfs)
        // 因残留累积而耗尽空间（曾观察到 119M 残留）。
        if tokio::fs::try_exists(SINGBOX_UPGRADE_TEMP_DIR)
            .await
            .unwrap_or(false)
        {
            tokio::fs::remove_dir_all(SINGBOX_UPGRADE_TEMP_DIR)
                .await
                .context("清理旧的 sing-box 升级暂存目录失败")?;
        }
        fs::create_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await?;
        let archive_path = format!("{}/sing-box.tar.gz", SINGBOX_UPGRADE_TEMP_DIR);
        SingBoxInstaller::download_file(&release.download_url, &archive_path).await?;

        // 完整性校验（方案 A）：先验 SHA256，再确认 GitHub 为该 digest 存有 attestation。
        // 上游 sing-box 不提供 minisign，但 GitHub 为每个 release 生成原生
        // sigstore attestation；此处以「该 digest 是否有 attestation 记录」作为信任锚。
        if let Err(e) = manager
            .verify_download(&archive_path, &release.sha256)
            .await
        {
            // 校验失败即删除已下载文件，避免残留被后续步骤误用。
            tokio::fs::remove_file(&archive_path).await.ok();
            return Err(e);
        }

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
        // 通过“暂存文件 + 原子 rename”替换（禁止直接 fs::copy 覆盖正在运行的二进制，
        // 否则会触发 ETXTBSY 导致“复制 sing-box 二进制失败”）。
        SingBoxInstaller::replace_binary(&unpacked_bin, singbox::BIN).await?;

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
        assert_eq!(
            parse_version_from_output(out),
            Some("1.14.0-rc.4".to_string())
        );
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

    // ---- 完整性校验（方案 A）：SHA256 + GitHub attestation 存在性 ----

    #[test]
    fn test_attestation_path_is_wellformed() {
        // 锁定 API 路径形态：/repos/{owner}/{repo}/attestations/sha256:<hex>
        let p =
            attestation_path("2375de6999f4f56ab46b4fc5ddf26a6aba1d3e61a0f4e7ddec2f4690457d5f63");
        assert_eq!(
            p,
            "/repos/SagerNet/sing-box/attestations/sha256:2375de6999f4f56ab46b4fc5ddf26a6aba1d3e61a0f4e7ddec2f4690457d5f63"
        );
    }

    #[test]
    fn test_attestation_path_rejects_non_sha256() {
        assert!(attestation_path("").is_empty());
        assert!(attestation_path("not-a-hash").is_empty());
        // 长度不足 / 含非法字符 → 视为无效，不得发请求
        assert!(attestation_path("abc").is_empty());
        assert!(attestation_path(&"z".repeat(64)).is_empty());
    }

    #[test]
    fn test_attestation_path_uppercase_is_lowercased() {
        // 上游 digest 可能给大写；路径必须以小写 hex 请求。
        let p = attestation_path(&"A".repeat(64));
        assert!(p.ends_with(&"a".repeat(64)), "路径应小写化, got: {}", p);
    }

    // ---- attestation 完整 URL（守卫：double-`/repos` 回归）----

    #[test]
    fn test_attestation_url_is_complete_and_has_single_repos() {
        let hex = "2375de6999f4f56ab46b4fc5ddf26a6aba1d3e61a0f4e7ddec2f4690457d5f63";
        let url = attestation_url(hex);

        // 完整字面量：任何拼接错误（如双 /repos、丢主机名）都会在此爆掉。
        assert_eq!(
            url,
            "https://api.github.com/repos/SagerNet/sing-box/attestations/sha256:2375de6999f4f56ab46b4fc5ddf26a6aba1d3e61a0f4e7ddec2f4690457d5f63"
        );
        // 防复发守卫：路径中只能出现一处 `/repos`。
        assert_eq!(
            url.matches("/repos").count(),
            1,
            "URL 只能含一处 /repos, got: {}",
            url
        );
        assert!(
            !url.contains("/repos/repos"),
            "不得出现双重 /repos: {}",
            url
        );
    }

    #[test]
    fn test_attestation_url_empty_for_invalid_hex() {
        assert!(attestation_url("").is_empty());
        assert!(attestation_url("not-a-hash").is_empty());
        assert!(attestation_url("abc").is_empty());
        assert!(attestation_url(&"z".repeat(64)).is_empty());
    }

    #[test]
    fn test_attestation_url_lowercases_and_keeps_single_repos() {
        let url = attestation_url(&"A".repeat(64));
        assert!(url.ends_with(&"a".repeat(64)), "应小写: {}", url);
        assert_eq!(url.matches("/repos").count(), 1, "单 /repos, got: {}", url);
    }

    // ---- attestation 判定分支（纯函数，无网络）----

    #[test]
    fn test_attestation_decision_404_is_false() {
        assert!(!attestation_decision(404, None).unwrap());
    }

    #[test]
    fn test_attestation_decision_empty_array_is_false() {
        let body = serde_json::json!({ "attestations": [] });
        assert!(!attestation_decision(200, Some(&body)).unwrap());
    }

    #[test]
    fn test_attestation_decision_missing_field_is_false() {
        let body = serde_json::json!({});
        assert!(!attestation_decision(200, Some(&body)).unwrap());
    }

    #[test]
    fn test_attestation_decision_nonempty_array_is_true() {
        let body = serde_json::json!({ "attestations": [{ "bundle": {} }] });
        assert!(attestation_decision(200, Some(&body)).unwrap());
    }

    #[test]
    fn test_attestation_decision_server_error_is_err() {
        assert!(attestation_decision(500, None).is_err());
        assert!(attestation_decision(403, None).is_err());
        assert!(attestation_decision(301, None).is_err());
    }

    #[test]
    fn test_attestation_decision_200_without_body_is_err() {
        // 2xx 但拿不到响应体 → 无法判定。必须 Err（fail-closed），
        // 不得当作 false 静默放过。
        assert!(attestation_decision(200, None).is_err());
    }

    #[tokio::test]
    async fn test_verify_sha256_file_detects_mismatch() {
        let dir = std::env::temp_dir().join(format!("sbv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("data.bin");
        std::fs::write(&f, b"hello").unwrap();

        // 正确值：sha256("hello")
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256_file(f.to_str().unwrap(), good).await.is_ok());

        // 错误值必须失败
        let bad = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_sha256_file(f.to_str().unwrap(), bad).await.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_verify_sha256_file_is_case_insensitive() {
        let dir = std::env::temp_dir().join(format!("sbv-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("data.bin");
        std::fs::write(&f, b"hello").unwrap();

        let upper = "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824";
        assert!(verify_sha256_file(f.to_str().unwrap(), upper).await.is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_verify_sha256_file_errors_on_missing_file() {
        assert!(
            verify_sha256_file("/nonexistent/sing-box.tar.gz", &"a".repeat(64))
                .await
                .is_err(),
            "文件缺失应返回错误，而非 PANIC"
        );
    }

    #[test]
    fn test_find_asset_sha256_extracts_and_rejects() {
        let sha = "b".repeat(64);
        let assets = vec![
            test_asset("other.tar.gz", Some(&format!("sha256:{}", "a".repeat(64)))),
            test_asset("want.tar.gz", Some(&format!("sha256:{}", sha))),
            test_asset("nodigest.tar.gz", None),
            test_asset("short.tar.gz", Some("sha256:abc")),
        ];
        assert_eq!(find_asset_sha256(&assets, "want.tar.gz"), Some(sha));
        assert_eq!(find_asset_sha256(&assets, "missing.tar.gz"), None);
        // 无 digest 字段 → None（调用方据此报错，不得静默放行）
        assert_eq!(find_asset_sha256(&assets, "nodigest.tar.gz"), None);
        // digest 长度非法 → None
        assert_eq!(find_asset_sha256(&assets, "short.tar.gz"), None);
    }

    fn test_asset(
        name: &str,
        digest: Option<&str>,
    ) -> crate::core::network::release_api::ReleaseAsset {
        crate::core::network::release_api::ReleaseAsset {
            name: name.to_string(),
            browser_download_url: String::new(),
            url: String::new(),
            size: None,
            digest: digest.map(|d| d.to_string()),
        }
    }
}
