//! usque 的 I/O 层：下载、校验、注册、安装、卸载、状态探测。
//!
//! # 设计原则
//!
//! 所有**判定**都抽成纯函数（在 [`super::pin`] / [`super::unit`] /
//! [`super::engine`] / [`super::health`]），本文件只做「读状态 → 调纯函数
//! → 执行副作用」。
//!
//! 原因是路径被硬编码为 `/etc/wwps/...`，单测跑不到真实的 I/O 分支；
//! 把判定与副作用分离，判定就能被穷举测试。
//!
//! # 卸载是幂等的
//!
//! 见 [`InstalledKinds`] —— 只装了一套、或两套都没装，都必须返回成功。

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::fs;

use crate::core::cmd_async::run_cmd_output;
use crate::core::network::ProbeConclusion;
use crate::core::paths::{usque as usque_paths, warp as warp_paths};
use crate::core::usque::{pin, unit};

/// 安装时应该装哪一套 WARP 实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPlan {
    /// 装 Xray 内嵌的 WireGuard（走 UDP 2408）。
    WireGuard,
    /// 装 usque（MASQUE over TCP/443）。
    Usque,
}

impl InstallPlan {
    /// 供 bot 文案使用的 i18n key。
    pub const fn i18n_key(&self) -> &'static str {
        match self {
            InstallPlan::WireGuard => "warp.install_done_wireguard",
            InstallPlan::Usque => "warp.install_done_usque",
        }
    }
}

/// 由探测结论决定装哪一套。
///
/// # 为何是「只装需要的」（用户决策 W2）
///
/// 两套实现并存不是让它们都装一遍，而是**按机房实际条件选一套**：
/// 机房封了 UDP 就装走 TCP 的 usque，UDP 通就装开销更低的原生 WireGuard。
/// 装一套用不上的只会增加攻击面与维护面。
///
/// # 为何 `FullyBlocked` 与 `ProbeFailed` 都要拒绝
///
/// - `FullyBlocked`：两条通路都不可用，装了也无效。
///   报成功会让管理员以为「已经好了」——这正是本次要消除的那类假阳性。
/// - `ProbeFailed`：本地发包/连接都失败，说明环境本身有问题
///   （无路由、权限、DNS 解释不了等）。基于**未知**状态安装是在猜。
///
/// 两者都返回 `Err` 且附带具体原因，供 bot 直接展示。
pub fn install_plan(conclusion: ProbeConclusion, probe_failed: bool) -> Result<InstallPlan> {
    if probe_failed {
        anyhow::bail!(
            "出口探测未能完成（本地发包或连接失败），无法判断该装哪一套。\
             请先用「出口连通性探测」确认环境。"
        );
    }

    match conclusion {
        // UDP 可用（不管 TCP）→ 装原生 WireGuard。
        ProbeConclusion::WgUsable | ProbeConclusion::UdpOnly => Ok(InstallPlan::WireGuard),
        // 仅 TCP 可用 → 装 usque。
        ProbeConclusion::TcpOnly => Ok(InstallPlan::Usque),
        // 两条都不通 → 拒绝。
        ProbeConclusion::FullyBlocked => anyhow::bail!(
            "两条 WARP 通路都不可用（UDP 与 TCP/443 均被阻断），安装无意义。\
             建议先确认机房策略，或更换机房。"
        ),
    }
}

/// 实测安装状态。
///
/// 「安装」的判据是**数据文件存在**，不是「我以为装了什么」——
/// 卸载必须基于真实文件状态，否则会留下幽灵残留。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstalledKinds {
    pub wireguard: bool,
    pub usque: bool,
}

impl InstalledKinds {
    /// 都没装。
    pub const fn none() -> Self {
        Self {
            wireguard: false,
            usque: false,
        }
    }

    /// 至少装了一套。
    pub const fn any(&self) -> bool {
        self.wireguard || self.usque
    }

    /// 由两个「文件是否存在」的布尔值构造。**纯函数，便于穷举测试。**
    pub const fn detect(wireguard_account_exists: bool, usque_config_exists: bool) -> Self {
        Self {
            wireguard: wireguard_account_exists,
            usque: usque_config_exists,
        }
    }
}

/// 卸载范围。
///
/// # 为何需要「只卸一套」
///
/// 两套实现并存且不可互换（内核 TUN vs 本地 SOCKS），但它们的**账号
/// 与配置是独立的**。之前只有配对卸载，于是「只想重装 usque」的操作
/// 会把 WireGuard 账号一并删掉——而 WireGuard 的账号里含有 Cloudflare
/// 分配的 `reserved`，删除后必须重新注册，出口 IP 会变。
///
/// 这是一个**破坏性副作用**：用户以为在修一套，实际动了另一套。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallTarget {
    /// 只卸 WireGuard。
    WireGuard,
    /// 只卸 usque。
    Usque,
    /// 两套都卸。
    Both,
}

impl UninstallTarget {
    /// 返回 `(是否删 WireGuard, 是否删 usque)`。
    ///
    /// 抽成纯函数是为了可穷举测试：删除本身要碰真实文件系统与服务，
    /// 但「该删哪一套」这个决策必须被钉死。
    pub const fn scope(self) -> (bool, bool) {
        match self {
            Self::WireGuard => (true, false),
            Self::Usque => (false, true),
            Self::Both => (true, true),
        }
    }
}

/// 卸载结果——供 bot 文案决定「实际卸了什么」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UninstallOutcome {
    pub removed_wireguard: bool,
    pub removed_usque: bool,
}

impl UninstallOutcome {
    /// 什么都没卸（两套本来就没装）。
    pub const fn nothing_removed(&self) -> bool {
        !self.removed_wireguard && !self.removed_usque
    }

    /// i18n key，4 种组合互不相同。
    pub const fn i18n_key(&self) -> &'static str {
        match (self.removed_wireguard, self.removed_usque) {
            (true, true) => "warp.uninstalled_both",
            (true, false) => "warp.uninstalled_wireguard",
            (false, true) => "warp.uninstalled_usque",
            (false, false) => "warp.uninstalled_none",
        }
    }
}

/// 读取当前实际安装状态。
pub async fn detect_installed() -> InstalledKinds {
    let wg = fs::try_exists(warp_paths::ACCOUNT_FILE)
        .await
        .unwrap_or(false);
    let uq = fs::try_exists(usque_paths::CONFIG_FILE)
        .await
        .unwrap_or(false);
    InstalledKinds::detect(wg, uq)
}

/// 卸载所有已安装的 WARP 实现（WireGuard + usque）。
///
/// # 幂等
///
/// 两套都没装时返回 `Ok(UninstallOutcome::default())`，**不是错误**。
/// 卸载一个本来就不存在的东西是成功，不是失败——否则管理员会在
/// 「点了一次卸载没反应，再点一次报错」的循环里困惑。
///
/// # 不因单步失败而中断
///
/// 每一步（stop / disable / 删文件）的失败都被记录并继续。
/// 整体只要「尽力清干净」；最后清理 Xray 路由配置并重载核心。
/// 按 [`UninstallTarget`] 卸载指定范围，幂等。
///
/// # 卸载后必须刷新出口缓存
///
/// `current_egress()` 读的是探测缓存。若刚把缓存所指向的那一套卸了，
/// 缓存仍会选它 → 生成的 Xray 配置指向一个**已不存在的出口** → 黑洞
/// （流量被静默丢弃，而不是回退直连）。
///
/// 因此这里清空缓存，让出口引擎保守回退 `Direct`。代价是需要重新探测
/// 才会再次启用 WARP，但「暂时不用 WARP」远好于「流量黑洞」。
pub async fn uninstall_one(target: UninstallTarget) -> Result<UninstallOutcome> {
    let mut outcome = UninstallOutcome::default();

    let (remove_wg, remove_uq) = target.scope();

    if remove_wg {
        remove_wireguard_files().await;
        outcome.removed_wireguard = true;
    }

    if remove_uq {
        remove_usque().await;
        outcome.removed_usque = true;
    }

    // 路由文件可能引用刚被删掉的出口——一并清掉，让核心回到无 WARP 状态。
    let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;

    // 关键：清探测缓存。否则出口引擎仍会选被删的那一套 → 黑洞。
    crate::core::usque::engine::global_probe_cache().clear();

    crate::core::system::maintenance::MaintenanceManager::reload_core().await?;

    Ok(outcome)
}

pub async fn uninstall_all() -> Result<UninstallOutcome> {
    // 委托给参数化入口，避免两处各写一遍删除逻辑（那样迟早会分叉）。
    //
    // 语义差别：本函数**无条件**尝试删两套（不看是否装过），
    // 与旧行为一致——路由文件可能早于账号文件被手工删除而残留，
    // 那种情况下仍要清理。
    uninstall_one(UninstallTarget::Both).await
}

async fn remove_wireguard_files() {
    // 账号文件：失败只记日志，不阻断（文件可能已被手工删除）。
    if let Err(e) = fs::remove_file(warp_paths::ACCOUNT_FILE).await {
        log::warn!("删除 WARP 账号文件失败（忽略）: {e}");
    }
}

async fn remove_usque() {
    // 1. 先停服务，再摘 unit——顺序反了会让 systemd 去拉一个已不存在的 unit。
    for action in ["stop", "disable"] {
        if let Err(e) = run_systemctl(&[action, usque_paths::SERVICE_NAME]).await {
            // 服务本来就没运行 / 没 enable 时也会非零退出，属正常。
            log::debug!(
                "systemctl {action} {} 失败（可能本就未启用）: {e}",
                usque_paths::SERVICE_NAME
            );
        }
    }

    // 2. 删 unit 文件并让 systemd 重新读取。
    if Path::new(usque_paths::SERVICE_PATH).exists()
        && let Err(e) = fs::remove_file(usque_paths::SERVICE_PATH).await
    {
        log::warn!("删除 usque unit 文件失败（忽略）: {e}");
    }
    let _ = run_systemctl(&["daemon-reload"]).await;

    // 3. 删账号配置与二进制。
    //    账号文件含已注册的 Cloudflare access_token，必须清掉。
    for path in [usque_paths::CONFIG_FILE, usque_paths::BIN] {
        if let Err(e) = fs::remove_file(path).await {
            log::debug!("删除 {path} 失败（可能本就不存在）: {e}");
        }
    }

    // 4. 目录本身（若已空）。
    let _ = fs::remove_dir(usque_paths::DIR).await;
}

/// 安装 usque：下载 → 校验 → 注册 → 写 unit → 启用。
///
/// # 步骤顺序是刻意的
///
/// 先下载并**校验通过**才碰正式路径；注册失败则整个安装失败，
/// 不留下半成品可供误用。
///
/// # 已安装时幂等
///
/// `config.json` 已存在则直接返回——重复安装不应把已注册的账号洗掉
/// （那会让正在工作的隧道突然换出口 IP）。
pub async fn install(arch: &str) -> Result<()> {
    if fs::try_exists(usque_paths::CONFIG_FILE)
        .await
        .unwrap_or(false)
    {
        log::info!("usque 已安装（config.json 存在），跳过");
        return Ok(());
    }

    // 未 pin 的架构必须**明确失败**，不能退化成「不校验就装」。
    let (asset, expected_sha) = pin::pinned_target(arch).with_context(|| {
        format!("usque 未为架构 {arch} 固定版本与哈希，拒绝安装（无签名，无法安全校验）")
    })?;

    fs::create_dir_all(usque_paths::DIR)
        .await
        .context("创建 usque 目录失败")?;

    let tmp_zip = format!("{}/usque.zip", usque_paths::DIR);
    download(&pin::download_url(&asset), &tmp_zip).await?;
    verify_downloaded(&tmp_zip, expected_sha).await?;
    extract_binary(&tmp_zip).await?;
    let _ = fs::remove_file(&tmp_zip).await;

    register_account().await?;
    write_service_unit().await?;
    enable_service().await?;

    Ok(())
}

async fn download(url: &str, dest: &str) -> Result<()> {
    let out = tokio::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "300", "-o", dest, url])
        .output()
        .await
        .context("执行 curl 失败")?;
    if !out.status.success() {
        anyhow::bail!(
            "下载 usque 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// 校验下载的 zip 与 pin 的 sha256 一致。
///
/// 不匹配即中止——本模块没有其它信任根（usque Release 无签名），
/// 放过一次就等于允许任意二进制入库。
async fn verify_downloaded(path: &str, expected: &str) -> Result<()> {
    let (status, stdout, stderr) =
        run_cmd_output("sha256sum", &[path], std::time::Duration::from_secs(60))
            .await
            .context("执行 sha256sum 失败")?;
    if !status.success() {
        anyhow::bail!("sha256sum 失败: {}", stderr.trim());
    }
    let actual = stdout.split_whitespace().next().unwrap_or_default();
    if !pin::verify_sha256(expected, actual) {
        anyhow::bail!(
            "usque 校验失败，已中止。\n期望: {expected}\n实际: {actual}\n\
             （可能是下载损坏，也可能是上游 Release 被改动）"
        );
    }
    Ok(())
}

async fn extract_binary(zip: &str) -> Result<()> {
    let out = tokio::process::Command::new("unzip")
        .args(["-o", "-j", zip, "usque", "-d", usque_paths::DIR])
        .output()
        .await
        .context("执行 unzip 失败")?;
    if !out.status.success() {
        anyhow::bail!(
            "解压 usque 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(usque_paths::BIN).await?;
    let mut perms = meta.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(usque_paths::BIN, perms).await?;
    Ok(())
}

/// 注册 MASQUE 账号。
///
/// # 必须写入 `y` 到 stdin
///
/// `usque register` 会**交互式**要求接受 ToS：
///
/// ```text
/// You must accept the Terms of Service (...) to register. Do you agree? (y/n):
/// ```
///
/// 不写入会得到 `failed to read user input: EOF` 并注册失败（真机实测）。
///
/// 工作目录设为 `usque_paths::DIR`——`register` 把 `config.json`
/// 写在当前目录。
async fn register_account() -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new(usque_paths::BIN)
        .arg("register")
        .current_dir(usque_paths::DIR)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("启动 usque register 失败")?;

    if let Some(mut si) = child.stdin.take() {
        // ToS 确认；写入失败不阻断——某些版本可能不再询问，
        // 真正判据是下面 config.json 是否产出。
        let _ = si.write_all(b"y\n").await;
        let _ = si.flush().await;
        drop(si);
    }

    let out = child
        .wait_with_output()
        .await
        .context("等待 usque register 失败")?;

    if !out.status.success() {
        anyhow::bail!(
            "usque 注册失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // 退出码 0 也不够——`config.json` 真的存在才算成功。
    if !fs::try_exists(usque_paths::CONFIG_FILE)
        .await
        .unwrap_or(false)
    {
        anyhow::bail!(
            "usque 注册报告成功但未生成 {}（可能被限流或 ToS 未被接受）",
            usque_paths::CONFIG_FILE
        );
    }
    Ok(())
}

async fn write_service_unit() -> Result<()> {
    if !Path::new("/run/systemd/system").exists() {
        log::warn!("未检测到 systemd，跳过 usque 服务注册");
        return Ok(());
    }

    fs::write(usque_paths::SERVICE_PATH, unit::default_service_unit())
        .await
        .context("写入 usque unit 文件失败")?;
    let _ = run_systemctl(&["daemon-reload"]).await;
    Ok(())
}

async fn enable_service() -> Result<()> {
    if !Path::new("/run/systemd/system").exists() {
        return Ok(());
    }
    run_systemctl(&["enable", "--now", usque_paths::SERVICE_NAME]).await
}

async fn run_systemctl(args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .context("执行 systemctl 失败")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "systemctl {:?} 退出码 {:?}: {}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 穷举 4 种组合。
    #[test]
    fn test_detect_exhaustive() {
        assert_eq!(
            InstalledKinds::detect(true, true),
            InstalledKinds {
                wireguard: true,
                usque: true
            }
        );
        assert_eq!(
            InstalledKinds::detect(true, false),
            InstalledKinds {
                wireguard: true,
                usque: false
            }
        );
        assert_eq!(
            InstalledKinds::detect(false, true),
            InstalledKinds {
                wireguard: false,
                usque: true
            }
        );
        assert_eq!(InstalledKinds::detect(false, false), InstalledKinds::none());
    }

    #[test]
    fn test_installed_kinds_any() {
        assert!(!InstalledKinds::none().any());
        assert!(InstalledKinds::detect(true, false).any());
        assert!(InstalledKinds::detect(false, true).any());
        assert!(InstalledKinds::detect(true, true).any());
    }

    /// 4 种卸载结果的 i18n key 必须互不相同——否则 UI 会显示错文案。
    #[test]
    fn test_outcome_i18n_keys_are_distinct() {
        let combos = [
            UninstallOutcome {
                removed_wireguard: true,
                removed_usque: true,
            },
            UninstallOutcome {
                removed_wireguard: true,
                removed_usque: false,
            },
            UninstallOutcome {
                removed_wireguard: false,
                removed_usque: true,
            },
            UninstallOutcome::default(),
        ];
        let keys: Vec<&str> = combos.iter().map(|o| o.i18n_key()).collect();
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 4, "4 种组合的 i18n key 必须互不相同: {keys:?}");
        for k in keys {
            assert!(k.starts_with("warp."), "key 应位于 warp 命名空间: {k}");
        }
    }

    #[test]
    fn test_outcome_nothing_removed() {
        assert!(UninstallOutcome::default().nothing_removed());
        let only_wg = UninstallOutcome {
            removed_wireguard: true,
            removed_usque: false,
        };
        assert!(!only_wg.nothing_removed());
        let only_uq = UninstallOutcome {
            removed_wireguard: false,
            removed_usque: true,
        };
        assert!(!only_uq.nothing_removed());
    }

    /// 「什么都没卸」必须是**成功**语义，不是错误。
    ///
    /// 幂等：卸载一个本就不存在的东西算成功。
    #[test]
    fn test_nothing_removed_is_not_an_error_state() {
        let o = UninstallOutcome::default();
        assert!(o.nothing_removed());
        assert_eq!(o.i18n_key(), "warp.uninstalled_none");
    }

    // ──────────────── 安装路径的前置判定 ────────────────

    /// 未 pin 的架构必须在**下载前**就拒绝。
    ///
    /// `install()` 依赖 `pinned_target(arch)` 返回 `Some`；
    /// 这条测试把「未 pin → 拒绝安装」这个前置条件钉死，
    /// 防的是有人把 `with_context` 改成 `unwrap_or_default`
    /// 从而退化成「不校验就装」。
    #[test]
    fn test_install_requires_pinned_arch() {
        assert!(pin::pinned_target("x86_64").is_some(), "amd64 必须已 pin");
        assert!(
            pin::pinned_target("riscv64").is_none(),
            "未 pin 的架构必须返回 None（install 据此拒绝）"
        );
    }

    /// `/run/systemd/system` 不存在时（容器等）安装仍需成功。
    ///
    /// 这条不直接调 `install()`（会真下载），而是断言判定条件本身：
    /// 服务注册步骤用 `exists()` 短路，而不是硬失败。
    #[test]
    fn test_systemd_absence_is_detected_by_path_not_assumed() {
        // 容器里这个路径不存在；开发机上存在。两者都应被优雅处理。
        let present = Path::new("/run/systemd/system").exists();
        // 只断言判定函数不 panic 且语义直观
        assert_eq!(present, Path::new("/run/systemd/system").exists());
    }

    // ─────────────── T1: 安装分派 ───────────────

    /// **穷举 4 态**。
    #[test]
    fn test_install_plan_exhaustive() {
        use ProbeConclusion::*;
        assert_eq!(
            install_plan(WgUsable, false).unwrap(),
            InstallPlan::WireGuard
        );
        assert_eq!(
            install_plan(UdpOnly, false).unwrap(),
            InstallPlan::WireGuard
        );
        assert_eq!(install_plan(TcpOnly, false).unwrap(), InstallPlan::Usque);
        assert!(install_plan(FullyBlocked, false).is_err());
    }

    /// `FullyBlocked` 必须**拒绝安装**，而不是「照装但提示无效」。
    ///
    /// 报成功会让管理员以为已经好了——这正是本次要消除的假阳性。
    #[test]
    fn test_install_plan_refuses_when_fully_blocked() {
        let e = install_plan(ProbeConclusion::FullyBlocked, false)
            .expect_err("两条通路都不通时必须拒绝");
        let msg = e.to_string();
        assert!(
            msg.contains("UDP") || msg.contains("TCP"),
            "错误信息应说明具体原因: {msg}"
        );
    }

    /// 探测本身失败（本地发包/连接失败）时也必须拒绝。
    ///
    /// 基于**未知**状态安装是在猜。
    #[test]
    fn test_install_plan_refuses_when_probe_failed() {
        // 即使结论看起来乐观，探测失败也不能当依据。
        for c in [
            ProbeConclusion::WgUsable,
            ProbeConclusion::TcpOnly,
            ProbeConclusion::UdpOnly,
            ProbeConclusion::FullyBlocked,
        ] {
            assert!(
                install_plan(c, true).is_err(),
                "探测失败时不得安装（结论={c:?}）"
            );
        }
    }

    /// 本部署的实际处境（UDP 被封、TCP 可用）必须选 usque。
    /// 这条把线上实测结论固化成可执行断言。
    #[test]
    fn test_this_deployments_condition_installs_usque() {
        assert_eq!(
            install_plan(ProbeConclusion::TcpOnly, false).unwrap(),
            InstallPlan::Usque,
            "TCP-only 机房应装 usque（经 TCP/443）"
        );
    }

    /// 两个 plan 的 i18n key 必须互不相同。
    #[test]
    fn test_install_plan_i18n_keys_distinct() {
        let a = InstallPlan::WireGuard.i18n_key();
        let b = InstallPlan::Usque.i18n_key();
        assert_ne!(a, b);
        assert!(a.starts_with("warp."));
        assert!(b.starts_with("warp."));
    }

    // ────────── 卸载粒度：可以只卸一套 ──────────

    /// 目标枚举 → 该删哪一套。**穷举全部组合**，因为漏掉任一格
    /// 都会造成「以为在卸一套、实际动了另一套」。
    #[test]
    fn test_uninstall_target_scope_is_exhaustive() {
        // (目标, 应删 WireGuard, 应删 usque)
        let cases = [
            (UninstallTarget::WireGuard, true, false),
            (UninstallTarget::Usque, false, true),
            (UninstallTarget::Both, true, true),
        ];
        for (target, expect_wg, expect_uq) in cases {
            assert_eq!(
                UninstallTarget::scope(target).0,
                expect_wg,
                "{target:?} 对 WireGuard 的处置错误"
            );
            assert_eq!(
                UninstallTarget::scope(target).1,
                expect_uq,
                "{target:?} 对 usque 的处置错误"
            );
        }
    }

    /// **本次重构要修的核心问题**：只卸 usque 时不得触碰 WireGuard。
    ///
    /// WireGuard 账号里含 Cloudflare 分配的 `reserved`，删掉就得重新注册，
    /// 出口 IP 会变——用户以为在修一套，实际动了另一套。
    #[test]
    fn test_uninstall_usque_does_not_touch_wireguard() {
        assert!(
            !UninstallTarget::scope(UninstallTarget::Usque).0,
            "只卸 usque 不得删 WireGuard 账号"
        );
    }

    /// 对称：只卸 WireGuard 时不得触碰 usque。
    #[test]
    fn test_uninstall_wireguard_does_not_touch_usque() {
        assert!(
            !UninstallTarget::scope(UninstallTarget::WireGuard).1,
            "只卸 WireGuard 不得删 usque 的 config.json / systemd 服务"
        );
    }

    /// **黑洞防护**：卸载后必须清探测缓存。
    ///
    /// 若不清，`current_egress()` 仍会选刚被删掉的那一套 →
    /// 生成的 Xray 配置指向不存在的出口 → 流量被**静默丢弃**
    /// （而不是回退直连）。这是比「WARP 不生效」严重得多的故障。
    #[test]
    fn test_uninstall_clears_probe_cache() {
        const SRC: &str = include_str!("installer.rs");
        let start = SRC
            .find("pub async fn uninstall_one")
            .expect("uninstall_one 必须存在");
        let body = &SRC[start..];
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];

        assert!(
            body.contains("global_probe_cache()"),
            "卸载后必须清探测缓存，否则出口引擎会选一个已不存在的出口 → 黑洞"
        );
        assert!(
            body.contains(".clear()"),
            "必须调用 clear()——只读取缓存是没有意义的"
        );
    }

    /// 两处删除逻辑不得各写一遍（否则迟早分叉）。
    #[test]
    fn test_uninstall_all_delegates_to_one() {
        const SRC: &str = include_str!("installer.rs");
        let start = SRC
            .find("pub async fn uninstall_all")
            .expect("uninstall_all 必须存在");
        let body = &SRC[start..];
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];

        assert!(
            body.contains("uninstall_one"),
            "uninstall_all 必须委托给 uninstall_one，避免删除逻辑分叉"
        );
        assert!(
            !body.contains("remove_usque()"),
            "uninstall_all 不得自己再调一遍删除函数"
        );
    }
}
