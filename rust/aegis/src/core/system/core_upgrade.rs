use crate::common::{BotAdapter, MessageContent, MessageId as AegisMsgId, TargetId};
use crate::core::cmd_async::run_cmd_status;
use crate::core::crypto::minisign::{self, MINISIGN_ACTIVE_KEYS, MINISIGN_HISTORICAL_KEYS};
use crate::core::network::release_api::{
    ReleaseAsset, ReleaseResponse, extract_sha256_from_body, fetch_json_from_mirrors,
    fetch_prerelease, find_minisig_asset, parse_digest, parse_sha256_manifest,
};
use crate::core::paths::xray;
use crate::core::system::core_health::{
    self, PreflightOutcome, run_config_preflight, select_backups_to_delete,
};
use crate::core::utils::{
    format_download_progress, human_readable_size, is_same_version, should_report,
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use rust_i18n::t;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{File as StdFile, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::task;
use zip::ZipArchive;

const WWPS_CORE_DEFAULT_OWNER: &str = xray::DEFAULT_OWNER;
const WWPS_CORE_DEFAULT_REPO: &str = xray::DEFAULT_REPO;
const WWPS_CORE_DEFAULT_SERVICE: &str = xray::DEFAULT_SERVICE;
const WWPS_CORE_DEFAULT_INSTALL_DIR: &str = xray::DIR;
const WWPS_CORE_DEFAULT_TEMP_DIR: &str = xray::DEFAULT_TEMP_DIR;
const WWPS_CORE_DEFAULT_BACKUP_PREFIX: &str = xray::DEFAULT_BACKUP_PREFIX;

const WWPS_CORE_RELEASE_API_BASE: &str = "https://api.github.com/repos";

/// 配置预检（`xray run -test`）的超时。
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(60);

/// 备份保留份数。自动回滚只需要「本次升级刚产生的那份」，多留的只是为了
/// 应对升级后才发现问题、还想再退一步的场景；无限增长会吃光 VPS 磁盘。
const BACKUP_KEEP: usize = 3;

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
    pub minisig_url: Option<String>,
}

pub struct WwpsCoreUpgradeManager {
    config: Arc<WwpsCoreUpgradeConfig>,
    client: reqwest::Client,
    github_token: Option<String>,
}

const USER_AGENT_VALUE: &str = "wwps-runtime-updater/1.0";

/// 执行 `<core> version` 读取本机核心版本的超时。///
/// 版本读取是升级流程的**前置门**，必须快速失败：超时则视为“版本未知”，
/// 放行升级（行为退化为升级前的现状），绝不能因为一个辅助检查卡住升级。
const CURRENT_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// 从 `wwps-core version` 输出中解析版本号。
///
/// 目标机真实输出（上游 `core.VersionStatement()` 拼装）：
/// ```text
/// Xray 26.9.30 (Xray, Penetrates Everything.) b26a91d (go1.27.1 linux/amd64)
/// A unified platform for anti-censorship.
/// ```
/// 版本号为首个 `Xray ` 行里 `Xray` 之后的第一个空白分隔 token。上游版本号形态
/// 随发布策略变化（`1.8.x` → 日期式 `26.9.30`），故**不假设点分结构**。
pub fn parse_xray_version_from_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("Xray ")?;
        let version = rest.split_whitespace().next()?;
        (!version.is_empty()).then(|| version.to_string())
    })
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
            anyhow::bail!("Xray-core 安装目录不存在: {}", self.install_dir.display());
        }

        let binary_path = self.install_dir.join("wwps-core");
        if !binary_path.exists() {
            anyhow::bail!(
                "未找到 Xray-core 可执行文件，请先通过 install.sh 安装: {}",
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
        crate::bootstrap::install_crypto_provider();
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

    /// 读取本机已安装核心的版本号。
    ///
    /// 返回 `None` 表示**版本未知**（二进制缺失、执行失败、超时、输出无法解析），
    /// 调用方应据此放行升级，不得据此阻断。
    pub async fn current_version(&self) -> Option<String> {
        let binary = self.config.install_dir.join("wwps-core");
        let output = tokio::time::timeout(
            CURRENT_VERSION_TIMEOUT,
            tokio::process::Command::new(&binary)
                .arg("version")
                // 超时后 tokio 会 drop 未来的 Child；不开此选项，挂死的 `version`
                // 子进程会变成孤儿继续占用 CPU 与内存。
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        parse_xray_version_from_output(&String::from_utf8_lossy(&output.stdout))
    }

    pub async fn fetch_recent_tags(&self, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }

        let config = &self.config;
        let path = format!(
            "{}/{}/releases?per_page={}",
            config.owner, config.repo, limit
        );
        let bases = vec![WWPS_CORE_RELEASE_API_BASE.to_string()];

        let releases: Vec<ReleaseResponse> =
            fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
                .await?;

        Ok(releases
            .into_iter()
            .map(|r| r.tag_name)
            .take(limit)
            .collect())
    }

    pub async fn fetch_release(&self, tag: Option<&str>) -> Result<WwpsCoreReleaseInfo> {
        let config = &self.config;
        let bases = vec![WWPS_CORE_RELEASE_API_BASE.to_string()];

        let release: ReleaseResponse = if let Some(t) = tag {
            let path = format!("{}/{}/releases/tags/{}", config.owner, config.repo, t);
            fetch_json_from_mirrors(&self.client, &bases, &path, self.github_token.as_deref())
                .await?
        } else {
            let path = format!("{}/{}/releases?per_page=20", config.owner, config.repo);
            fetch_prerelease(&self.client, &bases, &path, self.github_token.as_deref()).await?
        };

        let asset_name = format!("{}.zip", config.arch.asset_basename());
        let asset = match release.assets.iter().find(|a| a.name == asset_name) {
            Some(a) => a,
            None => {
                anyhow::bail!("未在 Release 中找到资产 {}", asset_name);
            }
        };

        let download_url = asset.download_url().to_string();
        if download_url.is_empty() {
            anyhow::bail!("Release 资产无下载地址");
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

        let minisig_url =
            find_minisig_asset(&release.assets, &asset.name).map(|a| a.download_url().to_string());

        Ok(WwpsCoreReleaseInfo {
            tag_name: release.tag_name,
            download_url,
            sha256,
            size: asset.size,
            minisig_url,
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

        fs::create_dir_all(&self.config.temp_dir)
            .await
            .context("创建临时目录失败")?;

        let response = self
            .build_request(&release.download_url)
            .send()
            .await
            .context("下载 Xray-core Release 失败")?
            .error_for_status()
            .context("Xray-core Release 下载返回错误状态")?;

        let total_size = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = fs::File::create(&temp_file)
            .await
            .context("创建 Xray-core 临时包失败")?;
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
                .context("写入 Xray-core 临时包失败")?;
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

        writer.flush().await.context("刷新 Xray-core 临时包失败")?;
        drop(writer);
        file.sync_all().await.context("同步 Xray-core 包失败")?;

        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != release.sha256 {
            fs::remove_file(&temp_file).await.ok();
            anyhow::bail!(
                "Xray-core 包 SHA256 校验失败，期望: {} 实际: {}",
                release.sha256,
                actual_hash
            );
        }

        // Minisign verification（条件启用：**存在签名则必须验证通过**）。
        //
        // 与 upgrade.rs 不同，本路径刻意不在签名缺失时报错。原因：
        // core（Xray-core）的默认 release 源是**上游** XTLS/Xray-core，
        // 而上游不提供任何 minisign 签名（实测其 release 有 64 个资产、
        // 0 个 .minisig，只有 .dgst）。本项目无权也无能力为它签。
        // 若在此硬性要求签名，会导致 core 安装与升级 100% 失败。
        //
        // 因此这里的策略是「存在即强校验」：一旦上游/镜像提供了签名，
        // 验证失败（甚至是版本不符）一律拒绝，不得回退到 SHA256 放行。
        // core 链路的完整性目前仅依赖 SHA256 digest（上游无签名可用）。
        //
        // ⚠️ 不要为了「与 upgrade.rs 统一」而把它改成硬校验。
        let Some(sig_url) = release.minisig_url.as_ref() else {
            log::warn!(
                "Release {} 未提供 Minisign 签名，仅用 SHA256 校验（上游无签名）",
                release.tag_name
            );
            return Ok(temp_file);
        };
        {
            let sig_bytes = self
                .build_request(sig_url)
                .send()
                .await
                .context("下载 Minisign 签名文件失败")?
                .error_for_status()
                .context("Minisign 签名文件下载失败")?
                .bytes()
                .await
                .context("读取 Minisign 签名文件失败")?;

            let sig_str =
                std::str::from_utf8(&sig_bytes).context("Minisign 签名不是有效的 UTF-8")?;
            let download_data =
                std::fs::read(&temp_file).context("读取下载文件用于 Minisign 验证失败")?;
            let info = minisign::verify_minisign(
                &download_data,
                sig_str,
                MINISIGN_ACTIVE_KEYS,
                MINISIGN_HISTORICAL_KEYS,
            )
            .map_err(|e| anyhow!("Minisign 验证失败: {}", e))?;

            let (got_version, got_asset) = minisign::parse_trusted_comment(&info.trusted_comment)?;
            // 精确相等，理由同 upgrade.rs：子串/前缀匹配可被伪造版本号绕过
            if got_version != release.tag_name {
                fs::remove_file(&temp_file).await.ok();
                anyhow::bail!(
                    "Minisign 版本不匹配: 期望 {}, 实际 {}",
                    release.tag_name,
                    got_version
                );
            }
            let expected_asset_name = format!("{}.zip", self.config.arch.asset_basename());
            if got_asset != expected_asset_name {
                fs::remove_file(&temp_file).await.ok();
                anyhow::bail!(
                    "Minisign 文件名不匹配: 期望 {}, 实际 {}",
                    expected_asset_name,
                    got_asset
                );
            }
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
            .with_context(|| format!("备份 Xray-core 核心失败: {}", core_path.display()))?;

        for data in ["geoip.dat", "geosite.dat"] {
            let src = self.config.install_dir.join(data);
            if src.exists() {
                let dst = backup_path.join(data);
                let _ = tokio::fs::copy(&src, &dst).await;
            }
        }

        Ok(backup_path)
    }

    /// 用**新核心**验证**现网配置**能否加载（升级前预检）。
    ///
    /// 这是零风险防线：失败时现网二进制根本没被动过。
    /// 上游会主动移除旧配置项（sing-box 迁移文档逐条标注移除版本；Xray
    /// v24.9.30 移除了 QUIC/DomainSocket 与远古配置兼容代码），因此
    /// 「旧配置 + 新核心」失败是**预期内**行为，不是边缘情况。
    ///
    /// 上游若重命名/移除 `-test` flag，预检会不可用——此时必须 fail-open
    /// 放行（见 `is_preflight_unsupported`），绝不能因预检不可用而永久阻断升级。
    pub async fn preflight_check_config(&self, new_binary: &Path) -> Result<PreflightOutcome> {
        let conf_dir = xray::CONF_DIR.to_string();
        run_config_preflight(
            new_binary,
            &["run", "-test", "-confdir", &conf_dir],
            PREFLIGHT_TIMEOUT,
        )
        .await
    }

    /// 从备份目录恢复核心二进制与 geo 数据，并重启服务。
    ///
    /// 沿用 `replace_core` 已验证的「暂存 `.new` + 原子 rename」模式：直接
    /// `fs::copy` 覆盖正在被映射执行的 ELF 会拿到 `ETXTBSY`。
    pub async fn restore_backup(&self, backup: &Path) -> Result<()> {
        let backup_core = backup.join("wwps-core");
        if !backup_core.exists() {
            anyhow::bail!("备份中未找到核心文件: {}", backup_core.display());
        }

        self.install_binary_from(&backup_core, "wwps-core").await?;

        for data in ["geoip.dat", "geosite.dat"] {
            let src = backup.join(data);
            if src.exists() {
                // geo 数据恢复失败不应阻断回滚：核心能起来才是第一优先级。
                if let Err(err) = self.install_data_file(&src, data).await {
                    log::warn!("恢复 {data} 失败（忽略）: {err}");
                }
            }
        }

        self.restart_service().await?;
        self.verify_service_active().await?;

        // 回滚后必须复验：unit 刚 exec 时也是 active，单次 is-active 会误报。
        // 不复验就等于向管理员宣称“服务已恢复”，而实际可能正在崩溃循环。
        let verdict =
            core_health::wait_for_health(&format!("{}.service", self.config.service_name)).await;
        if verdict.should_rollback() {
            anyhow::bail!("回滚后核心仍未健康（{verdict:?}）");
        }
        Ok(())
    }

    /// 裁剪备份目录：仅保留最近 `keep` 份，其余删除（最旧优先）。
    pub async fn prune_backups(&self, keep: usize) -> Result<Vec<PathBuf>> {
        let mut names = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.config.backup_dir).await {
            Ok(entries) => entries,
            // 备份目录不存在 = 还没备份过，无需裁剪。
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(err) => return Err(err).context("读取备份目录失败"),
        };

        while let Some(entry) = entries.next_entry().await.context("遍历备份目录失败")? {
            if entry.path().is_dir() {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
        }

        let deleted = select_backups_to_delete(&names, WWPS_CORE_DEFAULT_BACKUP_PREFIX, keep);
        let mut removed = Vec::new();
        for name in deleted {
            let path = self.config.backup_dir.join(&name);
            // 删除失败只记日志：备份多留几份不致命，不能因此中断升级。
            match fs::remove_dir_all(&path).await {
                Ok(()) => removed.push(path),
                Err(err) => log::warn!("删除旧备份 {} 失败: {}", path.display(), err),
            }
        }
        Ok(removed)
    }

    /// 把一个可执行文件装到 install_dir：暂存 → 0755 → 原子 rename。
    async fn install_binary_from(&self, source: &Path, file_name: &str) -> Result<()> {
        let staging = self.config.install_dir.join(format!("{file_name}.new"));
        fs::copy(source, &staging)
            .await
            .with_context(|| format!("复制到暂存文件失败: {}", staging.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&staging).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&staging, perms)
                .await
                .context("设置可执行权限失败")?;
        }

        let target = self.config.install_dir.join(file_name);
        fs::rename(&staging, &target).await.with_context(|| {
            format!(
                "原子替换失败: {} -> {}",
                staging.display(),
                target.display()
            )
        })
    }

    /// 恢复单个数据文件（geoip/geosite）：暂存 + 原子 rename。
    async fn install_data_file(&self, source: &Path, file_name: &str) -> Result<()> {
        let staging = self.config.install_dir.join(format!("{file_name}.new"));
        fs::copy(source, &staging)
            .await
            .with_context(|| format!("复制 {file_name} 到暂存文件失败"))?;
        let target = self.config.install_dir.join(file_name);
        fs::rename(&staging, &target)
            .await
            .with_context(|| format!("恢复 {file_name} 失败"))
    }

    pub async fn replace_core(&self, unpack_dir: &Path) -> Result<()> {
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
                .context("设置 Xray-core 可执行权限失败")?;
        }

        let final_target = self.config.install_dir.join("wwps-core");
        fs::rename(&target_core, &final_target)
            .await
            .context("替换 Xray-core 核心失败")?;

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
        let status = run_cmd_status("systemctl", &["is-active", &unit], Duration::from_secs(15))
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

    pub async fn run_upgrade(
        tag: Option<String>,
        adapter: &dyn BotAdapter,
        target: &TargetId,
    ) -> Result<()> {
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
        let service_unit = format!("{}.service", config.service_name);
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

        // 版本短路：与 Bot 自更新的 `is_current_version` 同构——本机已是目标版本时
        // 直接告知并返回，不进入下载/替换/重启。版本未知（`None`）则放行，
        // 行为退化为升级前的现状，辅助检查绝不阻断升级。
        if let Some(local) = manager.current_version().await
            && is_same_version(&local, &release.tag_name)
        {
            adapter
                .edit_message(
                    target,
                    &status_msg_id,
                    MessageContent {
                        text: t!(
                            "upgrade.core_already_latest",
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
            .unwrap_or_else(|| t!("upgrade.core_unknown_size").to_string());
        let info_text = t!(
            "upgrade.core_download_info",
            "0" => release.tag_name.as_str(),
            "1" => size_str.as_str(),
            "2" => release.sha256.as_str()
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
        let unpack_dir = manager.extract_archive(&archive_path).await?;

        // ── 防线 1：替换前预检（零风险）──
        // 用新核心验现网配置。不通过则完全不碰现网二进制——这是最安全的一层，
        // 因为此时备份尚未产生、服务未被重启。
        let new_binary = unpack_dir.join("xray");
        match manager.preflight_check_config(&new_binary).await? {
            PreflightOutcome::Passed => {
                let _ = adapter
                    .edit_message(
                        target,
                        &status_msg_id,
                        MessageContent {
                            text: t!("upgrade.core_preflight_ok").to_string(),
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
                            text: t!("upgrade.core_preflight_unsupported").to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
            PreflightOutcome::Invalid { reason } => {
                manager
                    .cleanup_paths(&[archive_path.clone(), unpack_dir.clone()])
                    .await;
                let text = t!("upgrade.core_preflight_failed", "0" => reason.as_str()).to_string();
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
        let backup_path = manager.backup_current_core().await?;

        // 备份后**立即**裁剪，而不是等到升级成功之后。
        //
        // 放在这里而不是成功路径末尾，是因为“只保留最近 N 份”这个不变式必须在
        // **任何结局**下都成立——健康检查失败触发回滚时会在 937 行早退，若把裁剪
        // 挂在成功分支末尾，回滚场景就会一份都不删，目录照样无限堆积
        // （真机验证：一次回滚后备份从 4 份涨到 5 份，287M）。
        // 自动回滚只需要本次刚产生的这一份，它必然落在保留集内。
        let pruned = manager.prune_backups(BACKUP_KEEP).await.unwrap_or_default();

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
        manager.replace_core(&unpack_dir).await?;

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

        manager.restart_service().await?;

        // 这里**故意不**用旧的单次 `verify_service_active` 作为门。
        // 它在 restart 后立即跑一次 `is-active`，而 Type=simple 的 unit 在进程
        // exec 那一刻就是 active；若新核心随后因配置不兼容退出，这次检查会失败并
        // 让 `run_upgrade` 直接返回 Err——**恰好把下面的健康检查与自动回滚全部
        // 跳过**，坏二进制留在原地。已实测（sleep 2 后 exit 1 的 unit）该单次检查
        // 会看到 active=true 而漏报；反过来在启动稍慢时又会误报失败并中断升级。
        // 因此只记日志，判定权交给下方跳崩溃周期的窗口化检查。
        if let Err(err) = manager.verify_service_active().await {
            log::warn!("restart 后首次 is-active 未通过（交由健康检查判定）: {err}");
        }

        // ── 防线 2：替换后健康检查 + 自动回滚 ──
        // 单次 is-active 会误判：Type=simple 在 exec 那一刻就 active，而配置
        // 解析失败发生在其后，且 Restart=always 会把它重新拉回 active。
        // 故跨崩溃周期采样 is-active + NRestarts。
        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_verifying").to_string(),
                    markup: None,
                },
            )
            .await;

        let verdict = core_health::wait_for_health(&service_unit).await;
        if verdict.should_rollback() {
            // 先回滚，再清理临时文件：无论回滚成败，压缩包/解压目录都不应残留。
            let result = Self::handle_rollback(
                &manager,
                &backup_path,
                verdict,
                adapter,
                target,
                &status_msg_id,
            )
            .await;
            manager
                .cleanup_paths(&[archive_path.clone(), unpack_dir.clone()])
                .await;
            return result;
        }

        manager
            .cleanup_paths(&[archive_path.clone(), unpack_dir.clone()])
            .await;

        let mut summary = t!(
            "upgrade.core_updated",
            "0" => release.tag_name.as_str(),
            "1" => backup_path.display().to_string().as_str()
        )
        .to_string();
        if verdict == core_health::HealthVerdict::Unknown {
            summary.push('\n');
            summary.push_str(t!("upgrade.core_verify_unknown").as_ref());
        }
        if !pruned.is_empty() {
            summary.push('\n');
            summary.push_str(&t!(
                "upgrade.core_pruned",
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

        Ok(())
    }

    /// 健康检查失败时的自动回滚：恢复**本次升级刚产生**的备份并复验。
    ///
    /// 回滚目标必须是本次的 `backup_path`，而不是重新扫目录取“最新备份”——
    /// 并发升级或目录残留会把它带到错误的版本上。
    #[allow(clippy::too_many_arguments)]
    async fn handle_rollback(
        manager: &WwpsCoreUpgradeManager,
        backup_path: &Path,
        verdict: core_health::HealthVerdict,
        adapter: &dyn BotAdapter,
        target: &TargetId,
        status_msg_id: &AegisMsgId,
    ) -> Result<()> {
        let _ = adapter
            .edit_message(
                target,
                status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_rolling_back").to_string(),
                    markup: None,
                },
            )
            .await;

        match manager.restore_backup(backup_path).await {
            Ok(()) => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!(
                                "upgrade.core_rollback_done",
                                "0" => format!("{verdict:?}").as_str()
                            )
                            .to_string(),
                            markup: None,
                        },
                    )
                    .await?;
                Ok(())
            }
            Err(err) => {
                // 回滚也失败：保留现场，不再重试，把备份路径交回管理员。
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!(
                                "upgrade.core_rollback_failed",
                                "0" => err.to_string().as_str(),
                                "1" => backup_path.display().to_string().as_str()
                            )
                            .to_string(),
                            markup: None,
                        },
                    )
                    .await?;
                Err(err)
            }
        }
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
            .build_request(manifest_asset.download_url())
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

    /// 用真实目录验证裁剪：用生产常量拼出的目录名，超出保留份数的会被删掉。
    ///
    /// 曾经只靠 `select_backups_to_delete` 的纯函数测试，而它用的是手写前缀、
    /// 且整条裁剪链从未在“回滚路径”上被执行过——真机上表现为一次回滚后备份
    /// 从 4 份涨到 5 份。这里直接跑 `prune_backups` 走完整链路。
    #[tokio::test]
    async fn test_prune_backups_uses_production_prefix_and_keeps_newest() {
        let tmp = tempdir().unwrap();
        let backup_dir = tmp.path().join("backup");
        std::fs::create_dir_all(&backup_dir).unwrap();

        // 刻意混入非备份目录，验证它不会被删。
        std::fs::create_dir_all(backup_dir.join("README.txt")).unwrap();
        std::fs::create_dir_all(backup_dir.join("wwps-core-backup-notatimestamp")).unwrap();

        for stamp in [
            "20250101000000",
            "20260930082914",
            "20261004023043",
            "20261004035431",
        ] {
            std::fs::create_dir_all(
                backup_dir.join(format!("{WWPS_CORE_DEFAULT_BACKUP_PREFIX}-{stamp}")),
            )
            .unwrap();
        }

        let config = WwpsCoreUpgradeConfig::new(
            "XTLS",
            "Xray-core",
            "wwps-core",
            tmp.path().to_path_buf(),
            backup_dir.clone(),
            tmp.path().join("temp"),
            CpuArch::Amd64,
        );
        let manager = WwpsCoreUpgradeManager::new(config).unwrap();

        let removed = manager.prune_backups(3).await.unwrap();
        assert_eq!(removed.len(), 1, "应只删最旧的一份，实际删除: {removed:?}");

        let left: Vec<String> = std::fs::read_dir(backup_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(left.contains(&"README.txt".to_string()), "外来文件被删了");
        assert!(
            left.contains(&"wwps-core-backup-notatimestamp".to_string()),
            "非法时间戳目录被删了"
        );
        // 不能按 `starts_with(prefix)` 计数：`wwps-core-backup-notatimestamp`
        // 同样以该前缀开头会被多算一份（第一次写这个断言就是这么错的）。
        // 必须只认「前缀 + 可解析时间戳」，并逐个比对保留下来的目录。
        let mut kept: Vec<String> = left
            .iter()
            .filter(|n| {
                n.strip_prefix(WWPS_CORE_DEFAULT_BACKUP_PREFIX)
                    .and_then(|r| r.strip_prefix('-'))
                    .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            })
            .cloned()
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            vec![
                "wwps-core-backup-20260930082914".to_string(),
                "wwps-core-backup-20261004023043".to_string(),
                "wwps-core-backup-20261004035431".to_string(),
            ],
            "保留的应当是最近 3 份"
        );
    }
    /// 目标机 `wwps-core version` 的真实输出。
    const XRAY_VERSION_OUTPUT: &str = "Xray 26.9.30 (Xray, Penetrates Everything.) b26a91d (go1.27.1 linux/amd64)\nA unified platform for anti-censorship.\n";

    #[test]
    fn test_parse_xray_version_from_output_typical() {
        assert_eq!(
            parse_xray_version_from_output(XRAY_VERSION_OUTPUT),
            Some("26.9.30".to_string())
        );
    }

    #[test]
    fn test_parse_xray_version_ignores_surrounding_noise() {
        let noisy = format!("warning: something\n{XRAY_VERSION_OUTPUT}");
        assert_eq!(
            parse_xray_version_from_output(&noisy),
            Some("26.9.30".to_string())
        );
    }

    #[test]
    fn test_parse_xray_version_from_output_rejects_invalid() {
        let cases = [
            ("", None),
            ("A unified platform for anti-censorship.\n", None),
            ("XrayCore 26.9.30\n", None),
            // 前缀存在但后面没有版本 token（空或纯空白）。
            ("Xray \n", None),
            ("Xray\n", None),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_xray_version_from_output(input),
                expected.map(str::to_string),
                "input={input:?}"
            );
        }
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
        std::fs::create_dir_all(&install_dir).unwrap();
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
}
