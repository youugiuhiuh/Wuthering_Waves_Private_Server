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
use crate::core::system::core_health::{
    self, PreflightOutcome, run_config_preflight, select_backups_to_delete,
};
use crate::core::utils::{human_readable_size, is_same_version};
use anyhow::{Context, Result, anyhow};
use rust_i18n::t;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

const SINGBOX_RELEASE_OWNER: &str = "SagerNet";
const SINGBOX_RELEASE_REPO: &str = "sing-box";
const SINGBOX_RELEASE_API_BASE: &str = "https://api.github.com/repos";
const SINGBOX_UPGRADE_TEMP_DIR: &str = "/tmp/sing-box-upgrade";

/// 执行 `wwps-box version` 读取本机版本的硬超时。
///
/// 版本读取是升级流程的**前置门**，必须快速失败：超时则视为“版本未知”，
/// 放行升级（行为退化为升级前的现状），绝不能因为一个辅助检查卡死整个升级。
/// 二进制损坏导致 `version` 子命令挂死是真实场景（此时既不能升级也不能卡住）。
const CURRENT_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// 从 GitHub release 页面 HTML 片段中解析某资产的 sha256 hex。
///
/// 用途：作为 attestation 不可达时的**降级**证据源。该端点
/// `github.com/{owner}/{repo}/releases/expanded_assets/{tag}` 是纯 HTML，
/// 不走 api.github.com、不计入 60/h 限流、无需认证。
///
/// 实现按 `<li class="Box-row">` **逐行切分**后取同一行内的文件名与 digest ——
/// 实测 167/167 与 API digest 完全一致；不能用全局正则乱配（会把邻居资产的
/// digest 错配给本资产，实测过大范围搜索确实会拿到错误值）。
fn parse_asset_digest_from_release_html(html: &str, asset_name: &str) -> Option<String> {
    for block in html.split("Box-row") {
        // 该行必须同时包含目标文件名与一个 sha256
        if !block.contains(asset_name) {
            continue;
        }
        // 文件名必须出现在下载链接里（避免注释/描述文本误命中）
        if !block.contains("/releases/download/") {
            continue;
        }
        if let Some(cap) = sha256_hex_in(block) {
            return Some(cap);
        }
    }
    None
}

/// 在给定字符串中取首个 64 位 hex sha256（可带 `sha256:` 前缀）。
fn sha256_hex_in(s: &str) -> Option<String> {
    const NEEDLE: &str = "sha256:";
    let mut start = 0usize;
    while let Some(pos) = s[start..].find(NEEDLE) {
        let hex_start = start + pos + NEEDLE.len();
        let candidate = s.get(hex_start..hex_start + 64)?;
        if candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(candidate.to_ascii_lowercase());
        }
        start = hex_start;
        if start >= s.len() {
            break;
        }
    }
    None
}

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

/// 纯决策：该 digest 是否被**降级证据链**接受。
///
/// 降级链路（方案 B）：attestation 不可达（限流/网络）时，回退到
/// 「release API 的 digest == release 页面 HTML 的 digest」。
/// 两者都是 GitHub 自己渲染的元数据，须**同时**拿到且相等才放行；
/// 任一缺失或不一致 → 拒绝（fail-closed）。
///
/// 安全代价（必须让调用方知晓并在日志中明确标注）：这弱于 attestation。
/// attestation 是 GitHub 用 Sigstore 对构建产物签发的证明，而 digest 元数据
/// 只表明「GitHub 当前展示的该资产 hash 是多少」。
fn degraded_accepts(api_digest: &str, html_digest: Option<&str>) -> bool {
    match html_digest {
        Some(h) => !api_digest.is_empty() && h.eq_ignore_ascii_case(api_digest),
        None => false,
    }
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

/// 该 403 是否属于 GitHub 的**速率限制**（而非权限/其他拒绝）。
///
/// 实测：GitHub 未认证请求超出 60 次/小时的主限流时返回 **403**（不是 429），
/// 响应体为 `{"message":"API rate limit exceeded for <ip>..."}`。
/// 若把它一律当致命错误，则任何与其他软件共享出口 IP 的机器都永远无法部署 ——
/// 这正是「attestation 查询返回 HTTP 403」一键部署失败的根因。
///
/// 判定依据（任一成立即视为限流）：
/// - 响应体 message 含 "rate limit"
/// - `x-ratelimit-remaining: 0`
/// - 存在 `retry-after`
fn is_rate_limited(
    status: u16,
    body_message: Option<&str>,
    ratelimit_remaining: Option<&str>,
    retry_after: Option<&str>,
) -> bool {
    if status == 429 {
        // 429 本身就是 Too Many Requests，无需额外证据。
        return true;
    }
    if status != 403 {
        return false;
    }
    if retry_after.is_some() {
        return true;
    }
    if ratelimit_remaining == Some("0") {
        return true;
    }
    body_message
        .map(|m| m.to_ascii_lowercase().contains("rate limit"))
        .unwrap_or(false)
}

/// 由限流响应头推算应等待的秒数（下限 1s，上限 60s，缺失 → 默认 5s）。
fn retry_delay_secs(retry_after: Option<&str>, reset_epoch: Option<&str>, now_epoch: u64) -> u64 {
    if let Some(ra) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return ra.clamp(1, 60);
    }
    if let Some(reset) = reset_epoch.and_then(|v| v.trim().parse::<u64>().ok()) {
        let wait = reset.saturating_sub(now_epoch);
        return wait.clamp(1, 60);
    }
    5
}

pub struct SingBoxUpgradeManager {
    client: reqwest::Client,
    github_token: Option<String>,
}

/// 配置预检（`sing-box check`）的超时。
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(60);

/// 备份保留份数，与 Xray-core 侧保持一致。
const BACKUP_KEEP: usize = 3;

/// Sing-box 备份目录与文件名前缀。
const SINGBOX_BACKUP_DIR: &str = "/etc/wwps/wwps-box/backup";
const SINGBOX_BACKUP_PREFIX: &str = "wwps-box-backup";

/// Sing-box 的 systemd unit 名。
const SINGBOX_SERVICE_UNIT: &str = "wwps-box.service";

/// Sing-box 配置预检与自动回滚。
///
/// 与 Xray-core 侧同构：先用新二进制验证现网配置（失败则不碰现网），替换后
/// 跨崩溃周期检查健康度，不健康则用本次备份自动回滚。
pub struct SingBoxBackupManager {
    backup_dir: PathBuf,
    binary: PathBuf,
}

impl SingBoxBackupManager {
    pub fn new() -> Self {
        Self {
            backup_dir: PathBuf::from(SINGBOX_BACKUP_DIR),
            binary: PathBuf::from(singbox::BIN),
        }
    }

    /// 备份当前二进制，返回备份目录。
    pub async fn backup_binary(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.backup_dir)
            .await
            .context("创建 Sing-box 备份目录失败")?;

        let backup_path = self.backup_dir.join(format!(
            "{}-{}",
            SINGBOX_BACKUP_PREFIX,
            chrono::Utc::now().format("%Y%m%d%H%M%S")
        ));
        fs::create_dir_all(&backup_path)
            .await
            .context("创建 Sing-box 备份子目录失败")?;

        let dst = backup_path.join("wwps-box");
        fs::copy(&self.binary, &dst)
            .await
            .with_context(|| format!("备份 Sing-box 核心失败: {}", self.binary.display()))?;

        Ok(backup_path)
    }

    /// 用新二进制验证现网配置。
    pub async fn preflight_check_config(&self, new_binary: &Path) -> Result<PreflightOutcome> {
        let conf_dir = singbox::CONF_DIR.to_string();
        run_config_preflight(new_binary, &["check", "-C", &conf_dir], PREFLIGHT_TIMEOUT).await
    }

    /// 从备份恢复二进制并重启服务。
    pub async fn restore_backup(&self, backup: &Path) -> Result<()> {
        let backup_core = backup.join("wwps-box");
        if !backup_core.exists() {
            anyhow::bail!("备份中未找到 Sing-box 二进制: {}", backup_core.display());
        }
        SingBoxInstaller::replace_binary(&backup_core, &self.binary).await?;
        SingBoxInstaller::restart_service().await?;

        // 回滚后复验：否则“服务已恢复”只是一句没有证据的断言。
        let verdict = core_health::wait_for_health(SINGBOX_SERVICE_UNIT).await;
        if verdict.should_rollback() {
            anyhow::bail!("回滚后 Sing-box 仍未健康（{verdict:?}）");
        }
        Ok(())
    }

    /// 裁剪备份目录：保留最近 `keep` 份。
    pub async fn prune_backups(&self, keep: usize) -> Result<Vec<PathBuf>> {
        let mut names = Vec::new();
        let mut entries = match fs::read_dir(&self.backup_dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(err) => return Err(err).context("读取 Sing-box 备份目录失败"),
        };

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("遍历 Sing-box 备份目录失败")?
        {
            if entry.path().is_dir() {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
        }

        let deleted = select_backups_to_delete(&names, SINGBOX_BACKUP_PREFIX, keep);
        let mut removed = Vec::new();
        for name in deleted {
            let path = self.backup_dir.join(&name);
            match fs::remove_dir_all(&path).await {
                Ok(()) => removed.push(path),
                Err(err) => log::warn!("删除旧 Sing-box 备份 {} 失败: {}", path.display(), err),
            }
        }
        Ok(removed)
    }
}

impl Default for SingBoxBackupManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SingBoxUpgradeManager {
    pub fn new() -> Result<Self> {
        crate::bootstrap::install_crypto_provider();
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
    ///
    /// 对 GitHub 限流（403/429）做有界重试：限流是**暂时性**错误，不代表该 digest
    /// 无 attestation；立即失败会让共享 IP 的机器永远无法部署。重试耗尽后仍 fail-closed。
    async fn has_attestation_for(&self, sha256_hex: &str) -> Result<bool> {
        const MAX_ATTEMPTS: usize = 4;
        let url = attestation_url(sha256_hex);
        if url.is_empty() {
            anyhow::bail!("无效的 sha256: {}", sha256_hex);
        }
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 1..=MAX_ATTEMPTS {
            let mut req = self
                .client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "wwps-runtime-updater/1.0");
            if let Some(t) = self.github_token.as_deref() {
                req = req.header("Authorization", format!("Bearer {}", t));
            }
            let resp = req.send().await.context("查询 attestation 失败")?;
            let status = resp.status().as_u16();

            if resp.status().is_success() {
                let json: serde_json::Value =
                    resp.json().await.context("解析 attestation 响应失败")?;
                return attestation_decision(status, Some(&json));
            }

            // 非 2xx：读取响应头/体以区分「限流」与「真拒绝」。
            let hdr = |name: &str| -> Option<String> {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            };
            let retry_after = hdr("retry-after");
            let remaining = hdr("x-ratelimit-remaining");
            let reset = hdr("x-ratelimit-reset");
            let body_msg = resp.text().await.ok();

            if is_rate_limited(
                status,
                body_msg.as_deref(),
                remaining.as_deref(),
                retry_after.as_deref(),
            ) {
                last_err = Some(anyhow!(
                    "attestation 查询返回 HTTP {} (GitHub 限流)",
                    status
                ));
                if attempt < MAX_ATTEMPTS {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    // GitHub 未认证主限流按小时重置，最多等 60s/轮也未必够；
                    // 若给出 retry-after 则优先用（通常很短）。
                    let wait = retry_delay_secs(retry_after.as_deref(), reset.as_deref(), now);
                    log::warn!(
                        "attestation 查询被限流 (HTTP {}), {}/{} 次, {}s 后重试",
                        status,
                        attempt,
                        MAX_ATTEMPTS,
                        wait
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                    continue;
                }
                break;
            }

            // 404 → 无记录；其他（权限等）→ fail-closed。
            return attestation_decision(status, None);
        }
        Err(last_err.unwrap_or_else(|| anyhow!("attestation 查询失败")))
    }

    /// 从 release 页面 HTML 片段取某资产的 digest（无认证、不计限流的降级证据源）。
    ///
    /// 端点实测可用：`github.com/{owner}/{repo}/releases/expanded_assets/{tag}`。
    /// 失败（网络/未找到）→ None，由调用方决定是否降级。
    async fn html_digest_for_asset(&self, tag: &str, asset_name: &str) -> Option<String> {
        let url = format!(
            "https://github.com/{}/{}/releases/expanded_assets/{}",
            SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO, tag
        );
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", "wwps-runtime-updater/1.0")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            log::warn!("release 页面 HTML 取 digest 失败: HTTP {}", resp.status());
            return None;
        }
        let html = resp.text().await.ok()?;
        parse_asset_digest_from_release_html(&html, asset_name)
    }

    /// 端到端完整性校验。
    ///
    /// **主路径（方案 A）**：`SHA256 文件 == 期望值` 且 GitHub 存有该 digest 的 attestation。
    ///
    /// **降级路径（方案 B）**：仅当 attestation 查询**不可达**（限流/网络错误，即暂时性
    /// 基础设施问题）时，回退到「release 页面 HTML 的 digest 与期望 digest 一致」。
    /// 无法降级的情形一律拒绝：
    /// - attestation 明确返回 404（说明 GitHub 没为这个 digest 签过）→ 拒绝，不降级；
    /// - HTML 取不到或与期望不一致 → 拒绝。
    ///
    /// 两条下载入径（首装 `installer.rs` 与升级 `upgrade.rs`）共用此函数。
    pub(crate) async fn verify_download_for_tag(
        &self,
        path: &str,
        sha256_hex: &str,
        tag: Option<&str>,
    ) -> Result<()> {
        verify_sha256_file(path, sha256_hex).await?;

        match self.has_attestation_for(sha256_hex).await {
            Ok(true) => Ok(()),
            Ok(false) => anyhow::bail!(
                "sing-box 完整性校验失败: {} 无 GitHub attestation 记录（可能被替换）",
                sha256_hex
            ),
            Err(e) => {
                // attestation 不可达（限流/网络）→ 尝试降级（方案 B）。
                let Some(tag) = tag else {
                    return Err(
                        e.context("attestation 查询不可达且无 tag 可用于降级校验（fail-closed）")
                    );
                };
                let version = tag.trim_start_matches('v');
                let arch = SingBoxInstaller::detect_arch()?;
                let asset_name = format!("sing-box-{}-linux-{}.tar.gz", version, arch);
                let html_digest = self.html_digest_for_asset(tag, &asset_name).await;
                if degraded_accepts(sha256_hex, html_digest.as_deref()) {
                    log::warn!(
                        "⚠️ sing-box 完整性校验已降级：attestation 不可达 ({}), 
已改用 release 页面 digest 交叉核对（{}）。安全性弱于 attestation，请向用户明确提示。",
                        e,
                        asset_name
                    );
                    return Ok(());
                }
                Err(e.context(format!(
                    "attestation 不可达且降级校验也未通过（HTML digest: {:?}），已拒绝安装（fail-closed）",
                    html_digest
                )))
            }
        }
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

    /// 读取本机已安装 Sing-box 的版本号。
    ///
    /// 返回 `None` 表示**版本未知**（二进制缺失、执行失败、超时、输出无法解析），
    /// 调用方应据此放行升级，不得据此阻断。
    pub async fn current_version() -> Option<String> {
        Self::query_version_with_timeout(Path::new(singbox::BIN), CURRENT_VERSION_TIMEOUT).await
    }

    /// 执行 `<binary> version` 并解析版本号，带硬超时。
    ///
    /// 二进制与超时均可注入，便于在不触碰真实安装目录的前提下测试挂死/缺失场景。
    /// `kill_on_drop(true)`：超时后 tokio 会 drop 未来的 Child，若不开启，挂死的
    /// 子进程会变成孤儿继续占用 CPU 与内存。
    pub async fn query_version_with_timeout(binary: &Path, timeout: Duration) -> Option<String> {
        let output = tokio::time::timeout(
            timeout,
            tokio::process::Command::new(binary)
                .arg("version")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        parse_version_from_output(&String::from_utf8_lossy(&output.stdout))
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

        // 版本短路：与 Bot 自更新的 `is_current_version` 同构——本机已是目标版本时
        // 直接告知并返回，不进入下载/替换/重启。版本未知（`None`）则放行。
        if let Some(local) = SingBoxUpgradeManager::current_version().await
            && is_same_version(&local, &release.tag_name)
        {
            adapter
                .edit_message(
                    target,
                    &status_msg_id,
                    MessageContent {
                        text: t!(
                            "menu.singbox_upgrade_already_latest",
                            "0" => local.as_str(),
                            "1" => release.tag_name.as_str()
                        )
                        .to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(());
        }

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

        // 完整性校验（方案 A 主路径 + 方案 B 降级）：先验 SHA256，再确认 GitHub 为该
        // digest 存有 attestation。上游 sing-box 不提供 minisign，但 GitHub 为每个
        // release 生成原生 sigstore attestation；attestation 不可达时降级为
        // 「release 页面 digest 与期望值一致」（带 tag 以定位 release 页面）。
        if let Err(e) = manager
            .verify_download_for_tag(&archive_path, &release.sha256, Some(&release.tag_name))
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

        let backup_manager = SingBoxBackupManager::new();

        // ── 防线 1：替换前预检（零风险）──
        // 用新二进制验现网配置。不通过则完全不碰现网二进制。
        match backup_manager
            .preflight_check_config(Path::new(&unpacked_bin))
            .await?
        {
            PreflightOutcome::Passed => {
                let _ = adapter
                    .edit_message(
                        target,
                        &status_msg_id,
                        MessageContent {
                            text: t!("menu.singbox_upgrade_preflight_ok").to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
            PreflightOutcome::Unsupported => {
                let _ = adapter
                    .edit_message(
                        target,
                        &status_msg_id,
                        MessageContent {
                            text: t!("menu.singbox_upgrade_preflight_unsupported").to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
            PreflightOutcome::Invalid { reason } => {
                let _ = fs::remove_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await;
                let text =
                    t!("menu.singbox_upgrade_preflight_failed", "0" => reason.as_str()).to_string();
                let _ = adapter
                    .edit_message(
                        target,
                        &status_msg_id,
                        MessageContent {
                            text: text.clone(),
                            markup: None,
                        },
                    )
                    .await;
                adapter
                    .send_message(target, MessageContent { text, markup: None })
                    .await?;
                anyhow::bail!("配置预检未通过，已中止升级（现网核心未被改动）");
            }
        }

        // 备份必须在替换**之前**产生：它是回滚的唯一来源。
        let backup_path = backup_manager.backup_binary().await?;

        // 备份后立即裁剪，而非等升级成功——与 Xray-core 侧同构：健康检查失败
        // 触发回滚时会提前返回，挂在成功分支末尾就永远裁不到。
        let pruned = backup_manager
            .prune_backups(BACKUP_KEEP)
            .await
            .unwrap_or_default();

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

        // ── 防线 2：替换后健康检查 + 自动回滚 ──
        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("menu.singbox_upgrade_verifying").to_string(),
                    markup: None,
                },
            )
            .await;

        let verdict = core_health::wait_for_health(SINGBOX_SERVICE_UNIT).await;
        if verdict.should_rollback() {
            let _ = adapter
                .edit_message(
                    target,
                    &status_msg_id,
                    MessageContent {
                        text: t!("menu.singbox_upgrade_rolling_back").to_string(),
                        markup: None,
                    },
                )
                .await;

            match backup_manager.restore_backup(&backup_path).await {
                Ok(()) => {
                    let _ = fs::remove_dir_all(SINGBOX_UPGRADE_TEMP_DIR).await;
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!(
                                    "menu.singbox_upgrade_rollback_done",
                                    "0" => format!("{verdict:?}").as_str()
                                )
                                .to_string(),
                                markup: None,
                            },
                        )
                        .await?;
                    return Ok(());
                }
                Err(err) => {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!(
                                    "menu.singbox_upgrade_rollback_failed",
                                    "0" => err.to_string().as_str(),
                                    "1" => backup_path.display().to_string().as_str()
                                )
                                .to_string(),
                                markup: None,
                            },
                        )
                        .await?;
                    return Err(err);
                }
            }
        }

        let mut summary =
            t!("menu.singbox_upgrade_success", "0" => release.tag_name.as_str()).to_string();
        if verdict == core_health::HealthVerdict::Unknown {
            summary.push('\n');
            summary.push_str(t!("menu.singbox_upgrade_verify_unknown").as_ref());
        }
        if !pruned.is_empty() {
            summary.push('\n');
            summary.push_str(&t!(
                "menu.singbox_upgrade_pruned",
                "0" => pruned.len().to_string().as_str()
            ));
        }

        adapter
            .send_message(
                target,
                MessageContent {
                    text: summary,
                    markup: None,
                },
            )
            .await?;

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

    /// 目标机 `wwps-box version` 的真实输出（多行、带 Tags/Revision/CGO）。
    /// 版本号只在首行，其后的 Tags 行极长且含逗号，解析器必须只认首行前缀。
    #[test]
    fn test_parse_version_from_output_real_multiline() {
        let out = "sing-box version 1.14.2\n\nEnvironment: go1.26.8 linux/amd64\nTags: with_gvisor,with_quic,with_dhcp,with_wireguard,with_utls,with_acme,with_clash_api,with_tailscale,with_ccm,with_ocm,with_cloudflared,with_naive_outbound,with_usbip,with_openvpn,with_openconnect,badlinkname,tfogo_checklinkname0,with_purego\nRevision: af6e64c3b69e6132ebaee0e1a3d24e93903f6709\nCGO: disabled\n";
        assert_eq!(parse_version_from_output(out), Some("1.14.2".to_string()));
    }

    #[test]
    fn test_parse_version_from_output_empty() {
        assert_eq!(parse_version_from_output(""), None);
        assert_eq!(parse_version_from_output("not a version line\n"), None);
    }

    /// 写一个假 `wwps-box`，仅回显版本行。
    #[cfg(unix)]
    fn write_fake_binary(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_query_version_reads_binary_output() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_fake_binary(
            tmp.path(),
            "wwps-box",
            "#!/bin/sh\necho 'sing-box version 1.14.2'\n",
        );

        assert_eq!(
            SingBoxUpgradeManager::query_version_with_timeout(
                &bin,
                std::time::Duration::from_secs(5)
            )
            .await,
            Some("1.14.2".to_string())
        );
    }

    /// 二进制损坏 / `version` 子命令挂死时，必须在超时内返回 `None`，
    /// 否则升级前的版本检查会把整个升级流程卡死。
    #[cfg(unix)]
    #[tokio::test]
    async fn test_query_version_times_out_on_hanging_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_fake_binary(tmp.path(), "wwps-box", "#!/bin/sh\nsleep 30\n");

        let started = std::time::Instant::now();
        let version = SingBoxUpgradeManager::query_version_with_timeout(
            &bin,
            std::time::Duration::from_millis(200),
        )
        .await;

        assert_eq!(version, None);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "超时未生效，耗时 {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_query_version_returns_none_for_missing_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("wwps-box");

        assert_eq!(
            SingBoxUpgradeManager::query_version_with_timeout(
                &bin,
                std::time::Duration::from_secs(5)
            )
            .await,
            None
        );
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

    /// 反向守卫：`SINGBOX_RELEASE_API_BASE` 已经**包含** `/repos`，
    /// `fetch_recent_tags` / `fetch_release` 把它与 `SagerNet/sing-box/releases...` 直接拼接。
    ///
    /// 若有人把常量改成裸主机 `https://api.github.com`（为“修” attestation 拼接而做的
    /// 反向改动），attestation URL 会**依然正确**（另一侧多补一个 `/repos` 即可），
    /// 9 个 attestation 测试全绿 —— 但 releases 路径会变成
    /// `https://api.github.com/SagerNet/sing-box/releases` → 404，
    /// 同一类 bug 换了个接缝复发。本测试锁住那个接缝。
    #[test]
    fn test_release_api_base_still_carries_repos_segment() {
        let url = format!(
            "{}/{}/{}/releases",
            SINGBOX_RELEASE_API_BASE, SINGBOX_RELEASE_OWNER, SINGBOX_RELEASE_REPO
        );
        assert_eq!(
            url,
            "https://api.github.com/repos/SagerNet/sing-box/releases"
        );
        assert_eq!(
            url.matches("/repos").count(),
            1,
            "releases URL 只能含一处 /repos: {}",
            url
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

    // ---- GitHub 限流识别（403 的两种含义）----

    #[test]
    fn test_is_rate_limited_detects_github_rate_limit_403() {
        // 实测响应体：GitHub 未认证限流返回 403 + "API rate limit exceeded for <ip>"
        let msg = "API rate limit exceeded for 1.2.3.4. (But here's the good news...)";
        assert!(is_rate_limited(403, Some(msg), None, None));
        // 429 同理
        assert!(is_rate_limited(429, None, None, None));
    }

    #[test]
    fn test_is_rate_limited_via_headers() {
        assert!(is_rate_limited(403, None, Some("0"), None));
        assert!(is_rate_limited(403, None, None, Some("30")));
    }

    #[test]
    fn test_is_rate_limited_false_for_real_403_and_404() {
        // 权限类 403（无 rate limit 特征）→ 不得当作限流，必须 fail-closed
        assert!(!is_rate_limited(
            403,
            Some("Resource not accessible"),
            Some("59"),
            None
        ));
        assert!(!is_rate_limited(404, None, None, None));
        assert!(!is_rate_limited(200, None, Some("0"), None));
        assert!(!is_rate_limited(401, Some("Bad credentials"), None, None));
    }

    #[test]
    fn test_retry_delay_secs_prefers_retry_after_and_clamps() {
        assert_eq!(retry_delay_secs(Some("12"), None, 0), 12);
        // 上限 60s，避免挂死部署
        assert_eq!(retry_delay_secs(Some("9999"), None, 0), 60);
        // 下限 1s
        assert_eq!(retry_delay_secs(Some("0"), None, 0), 1);
        // 回退到 x-ratelimit-reset（future → 差值）
        assert_eq!(retry_delay_secs(None, Some("1000"), 970), 30);
        // reset 已过期 / 缺失 → 默认 5s
        assert_eq!(retry_delay_secs(None, Some("900"), 1000), 1);
        assert_eq!(retry_delay_secs(None, None, 0), 5);
    }

    // ---- 方案 B：HTML digest 解析 + 降级决策 ----

    #[test]
    fn test_parse_asset_digest_scopes_to_matching_row() {
        // 两个 Box-row：必须取到与文件名同行的那个 digest，不得错配邻居。
        let other = "a".repeat(64);
        let want = "b".repeat(64);
        let html = format!(
            "<li class=\"Box-row\">\
               <a href=\"/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-arm64.tar.gz\">sing-box-1.14.0-linux-arm64.tar.gz</a>\
               <span>sha256:{}</span></li>\
             <li class=\"Box-row\">\
               <a href=\"/SagerNet/sing-box/releases/download/v1.14.0/sing-box-1.14.0-linux-amd64.tar.gz\">sing-box-1.14.0-linux-amd64.tar.gz</a>\
               <span>sha256:{}</span></li>",
            other, want
        );
        assert_eq!(
            parse_asset_digest_from_release_html(&html, "sing-box-1.14.0-linux-amd64.tar.gz"),
            Some(want)
        );
        assert_eq!(
            parse_asset_digest_from_release_html(&html, "sing-box-1.14.0-linux-arm64.tar.gz"),
            Some(other)
        );
        // 不在列表中的资产 → None（不得随便返回一个 digest）
        assert_eq!(
            parse_asset_digest_from_release_html(&html, "sing-box-1.14.0-linux-mips.tar.gz"),
            None
        );
        assert_eq!(parse_asset_digest_from_release_html("", "x.tar.gz"), None);
    }

    #[test]
    fn test_sha256_hex_in_rejects_short_and_uppercases() {
        assert_eq!(sha256_hex_in("sha256:abc"), None);
        assert_eq!(sha256_hex_in("no hash"), None);
        let upper = format!("sha256:{}", "A".repeat(64));
        assert_eq!(sha256_hex_in(&upper), Some("a".repeat(64)));
    }

    #[test]
    fn test_degraded_accepts_requires_matching_nonempty_digest() {
        let d = "a".repeat(64);
        // 两源一致 → 接受（含大小写不敏感）
        assert!(degraded_accepts(&d, Some(&d)));
        assert!(degraded_accepts(&d, Some(&d.to_ascii_uppercase())));
        // 不一致 → 拒绝
        assert!(!degraded_accepts(&d, Some(&"b".repeat(64))));
        // HTML 拿不到 → 拒绝（fail-closed，不得因证据缺失而放行）
        assert!(!degraded_accepts(&d, None));
        // 期望值为空 → 拒绝
        assert!(!degraded_accepts("", Some(&d)));
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
