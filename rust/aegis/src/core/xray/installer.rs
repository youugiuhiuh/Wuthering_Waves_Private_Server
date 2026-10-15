use crate::common::{BotAdapter, MessageContent, MessageId as AegisMsgId, TargetId};
use crate::core::paths::{warp as warp_paths, xray};
use crate::core::system::core_upgrade::{CpuArch, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use crate::core::system::maintenance::MaintenanceManager;
use anyhow::{Context, Result, anyhow};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Mutex;

const TOTAL_STEPS: u8 = 6;
const WWPS_CORE_TEMP_DIR: &str = "/tmp/wwps-core-installer";
const BASE_PACKAGES_APK: &[&str] = &[
    "sudo", "curl", "wget", "jq", "tar", "unzip", "bash", "openrc", "chrony",
];

pub struct WarpInstaller;

/// 两次 `rx_bytes` 采样的间隔。
///
/// 需要大于 WireGuard 的 keepalive 周期才有意义；Xray 侧配的是
/// `keepAlive: 30`（秒），但这里只判「有没有任何回包」，不过分依赖那个值。
/// 取 1s 是因为：若隧道可用，任何在途请求都会立刻产生回包；
/// 若不可用，等 30s 也只是自讨苦吃。
const WG_RX_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// wg0 隧道健康度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WgTunnelHealth {
    /// 采样期内 `rx_bytes` 有增长 → 真的在收包。
    Established,
    /// 计数不增长、或根本读不到 → 未建立。
    NoTraffic,
}

/// 读 `wg0` 的累计接收字节数；读不到返回 `None`。
///
/// 读不到是**正常**情形（接口还没建、容器里没这个路径），
/// 不应报错——调用方按「未建立」处理。
async fn read_wg_rx_bytes() -> Option<u64> {
    let raw = fs::read_to_string(crate::core::paths::warp::WG_RX_BYTES_PATH)
        .await
        .ok()?;
    raw.trim().parse::<u64>().ok()
}

impl WarpInstaller {
    /// 是否装有**任意一套** WARP 实现（WireGuard 或 usque）。
    ///
    /// # 为何必须查两套
    ///
    /// 两套实现的账号文件是**不同路径**：
    ///
    /// - WireGuard：`/etc/wwps/wwps-core/warp_account.json`
    /// - usque：`/etc/wwps/usque/config.json`
    ///
    /// 之前只查前者，于是「只装 usque」的机房（UDP 被封的机房正是这种）
    /// 会被判定为未安装，菜单显示「⚠️ 未检测到 Cloudflare WARP」并给出
    /// 安装按钮。管理员反复点击——每次都「成功」（幂等跳过）——
    /// 界面却永远说没装。这是一个无法自解的死循环。
    ///
    /// 出口条件不同的机房会装不同的实现，因此判定必须覆盖两套。
    pub async fn is_installed() -> bool {
        crate::core::usque::installer::detect_installed()
            .await
            .any()
    }

    /// 只装 WireGuard 那一套（注册账号 + 写路由，保留既有配置）。
    ///
    /// 从 [`Self::install`] 拆出，供「按探测结果分派」的安装路径
    /// （`install_smart`）单独调用。
    pub async fn install_wireguard_only() -> Result<()> {
        // 1. Register Account
        let config = crate::core::network::warp_api::register_account()
            .await
            .context("Failed to register WARP account")?;

        // 2. 先捕获现有配置，再覆写账号文件。
        //
        // 重装/升级不应抹掉管理员已配的域名清单与模式偏好。修复前这里
        // 无条件传 `vec![]` + `Default`，每次重装都会静默清空它们
        // （真机已复现：`proxy-warp + warp`（IPv4）退化为仅 `warp`，
        // 域名规则全丢）。
        let preserved = Self::read_preservable_config().await;

        // 3. Save Account Config
        let account_path = warp_paths::ACCOUNT_FILE;
        let content = serde_json::to_string_pretty(&config)?;
        fs::write(account_path, content).await?;

        // 4. Update core routing config
        //
        // scope 取新注册账号的默认值（`register_account` 已设好）——
        // 账号换新后把旧 scope 强行搬过来语义不对；而 `Domain` 是安全默认。
        let (rules, mode) = Self::config_to_restore(preserved, config.scope);
        crate::core::xray::config::ConfigManager::update_warp_routing_rules(
            rules,
            mode,
            config.scope,
            false,
        )
        .await?;

        Ok(())
    }

    /// 把一次探测结果写入出口引擎缓存。
    ///
    /// # 为何单独抽出
    ///
    /// 这一步是「安装后 WARP 能否生效」的关键：`current_egress()` 读的就是
    /// 这个缓存。之前**没有任何地方写它**，于是缓存永远为空、
    /// 出口引擎永远回退 `Direct` —— 即使 usque 装好了也不会启用。
    ///
    /// 抽成独立函数是为了可测：`install_smart` 本身要做真实探测与安装
    /// （无法在单测里跑），而「写缓存 → 出口引擎随之变化」这条因果关系
    /// 必须被断言住。
    pub fn record_probe_result(report: crate::core::network::ProbeReport) {
        crate::core::usque::engine::global_probe_cache()
            .store(report, crate::core::usque::engine::now_monotonic());
    }

    /// 按探测结果只装需要的那一套（TG 安装按钮的入口）。
    ///
    /// # 为何用**实时**探测而非缓存
    ///
    /// 安装是一次性决策，基于过期数据选错实现会留下一个装了却没用的组件。
    /// 探测只需 3s，完全可接受。
    ///
    /// # 安装后写缓存
    ///
    /// `current_egress()` 读的是 `global_probe_cache()`。若这里不写，
    /// 缓存会一直为空 → 出口引擎保守回退 `Direct` → 即使装好了也不会启用。
    /// 这正是「安装无调用点」之外的第二个断点。
    ///
    /// # 不做定时重探（用户决策 W3）
    ///
    /// 出口条件是机房级策略，不随时间漂移；再次启用只需**再点一次安装**
    /// （幂等：已装的会跳过）。
    pub async fn install_smart(arch: &str) -> Result<crate::core::usque::InstallPlan> {
        use crate::core::usque::installer::{InstallPlan, install_plan};

        // 1. 实时探测
        let report = crate::core::network::warp_probe::run_probe().await;

        // 探测是否「未能完成」而非「有结论」。
        //
        // `ProbeFailed`（本地发包失败）与 `Blocked`（静默超时）语义不同：
        // 前者说明环境有问题（无路由/权限），后者是机房真实阻断。
        // 只有当**两路都没拿到有效信号**时才视为探测失败。
        let udp_failed = report.udp == crate::core::network::UdpVerdict::ProbeFailed;
        let tcp_unusable = matches!(report.tcp, crate::core::network::TcpVerdict::Blocked);
        let probe_failed = udp_failed && tcp_unusable;

        // 2. 分派（拒绝时带具体原因）
        let plan =
            install_plan(report.conclusion, probe_failed).context("出口探测结果不支持安装 WARP")?;

        // 3. 只装需要的那一套
        match plan {
            InstallPlan::WireGuard => Self::install_wireguard_only().await?,
            InstallPlan::Usque => {
                crate::core::usque::installer::install(arch).await?;
            }
        }

        // 4. 写缓存——否则 current_egress() 永远读不到结论
        Self::record_probe_result(report);

        Ok(plan)
    }

    /// 兼容入口：只装 WireGuard。
    ///
    /// **TG 的安装按钮不走这里** —— 它走 `install_smart`（按探测结果分派）。
    /// 本函数保留给内部调用与既有测试。
    pub async fn install() -> Result<()> {
        Self::install_wireguard_only().await
    }

    /// 读现有路由配置供重装时保留；无配置时返回 `None`。
    ///
    /// 读失败一律当作「无既有配置」：宁可回到默认，也不应因一个损坏的
    /// 旧文件而让重装整个失败（重装本身是修复手段）。
    async fn read_preservable_config() -> Option<(Vec<String>, crate::core::xray::WarpMode)> {
        match crate::core::xray::ConfigManager::get_warp_routing_rules().await {
            Ok((rules, mode, _scope)) => Some((rules, mode)),
            Err(_) => None,
        }
    }

    /// 决定重装后应写回的路由配置。
    ///
    /// 纯函数，便于单测穷举两分支。
    ///
    /// - `preserved = Some(..)`：全新部署后的再次安装——原样保留。
    /// - `preserved = None`：首次安装——回到默认（空清单 + `Default`）。
    ///
    /// `scope` 不参与返回值：它不落在路由文件里，由调用方单独处理。
    fn config_to_restore(
        preserved: Option<(Vec<String>, crate::core::xray::WarpMode)>,
        _scope: crate::core::xray::WarpScope,
    ) -> (Vec<String>, crate::core::xray::WarpMode) {
        preserved.unwrap_or((Vec::new(), crate::core::xray::WarpMode::Default))
    }

    /// 由两次 `rx_bytes` 采样判定隧道健康度。
    ///
    /// 纯函数，便于单测穷举所有组合（含读不到的 `None`）。
    ///
    /// 判定用「增长」而非「> 0」：计数器是累计值，一旦通一次就永久为正，
    /// 用 `> 0` 会在「曾经通过、现在断了」时发出假阳性绿报。
    fn verdict_from_samples(first: Option<u64>, second: Option<u64>) -> WgTunnelHealth {
        match (first, second) {
            (Some(a), Some(b)) if b > a => WgTunnelHealth::Established,
            _ => WgTunnelHealth::NoTraffic,
        }
    }

    /// 卸载**所有** WARP 实现（WireGuard + usque），幂等。
    ///
    /// # 为什么两套一起卸
    ///
    /// 旧实现只清 WireGuard 的两个文件。Spec 2 落地后多出 usque 的
    /// 一整套资产（systemd unit、账号配置、二进制），只清一半会留下：
    ///
    /// - `wwps-usque.service` 仍在，systemd 仍会拉起它，端口仍被占；
    /// - 账号文件仍包着一个已注册的 Cloudflare 账号（含 `access_token`）；
    /// - Xray 还指着 `127.0.0.1:1080` 的 socks outbound——现在指向一个死端口。
    ///
    /// 而管理员看到的是「卸载成功」，无从察觉。
    ///
    /// # 幂等
    ///
    /// 只装了一套、或两套都没装，都返回 `Ok`——卸载一个不存在的东西
    /// 是成功，不是失败。
    pub async fn uninstall_paired() -> Result<crate::core::usque::UninstallOutcome> {
        crate::core::usque::installer::uninstall_all().await
    }

    /// 仅卸载 WireGuard 部分（保留供内部/兼容调用）。
    ///
    /// 对外（TG 卸载按钮）一律用 [`Self::uninstall_paired`]。
    pub async fn uninstall() -> Result<()> {
        let _ = fs::remove_file(warp_paths::ACCOUNT_FILE).await;
        let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    /// 重启 WARP。
    ///
    /// # 已废弃的语义陷阱
    ///
    /// 这个函数原名 `restart_service`，只做 `reload_core()`。对
    /// WireGuard 是对的（它跑在核心进程内），但名字让人以为它能重启
    /// **任何** WARP 服务——而 usque 是独立 systemd 服务，reload 核心
    /// 对它毫无作用。
    ///
    /// 线上故障正出在这里：点「重启服务」救卡住的 usque，什么都没发生，
    /// 界面却报成功。
    ///
    /// 现按实现分派，与子页按钮走同一条路径
    /// （[`crate::core::usque::installer::restart_one`]），避免两处语义分叉。
    ///
    /// **新代码应直接用 `restart_one(target)`** —— 它要求调用方显式
    /// 说明重启哪一套，不再有「不知道重启的是什么」的空间。
    pub async fn restart_service() -> Result<()> {
        crate::core::usque::installer::restart_one(
            crate::core::usque::installer::UninstallTarget::Both,
        )
        .await
    }

    /// 入口横幅的 i18n key——按**实际装了哪一套**分派。
    ///
    /// # 为何不能只用布尔值
    ///
    /// 两套是并存且不可互换的：WireGuard 用内核 TUN（`wg0`），usque 用
    /// 本地 SOCKS（`127.0.0.1:1080`），连 Xray outbound 的形状都不同
    /// （`wireguard` vs 扁平 `socks`）。所以界面必须说清楚当前装的是哪一套，
    /// 而不是笼统地讲「WARP 已安装」——后者会让管理员误以为走的是
    /// 内核接管那条路。
    ///
    /// 出口条件不同的机房会装不同的实现：UDP 可用装 WireGuard，
    /// UDP 被封装 usque。
    ///
    /// 纯函数，便于穷举四种组合而不用碰文件系统。
    pub const fn installed_banner_key(kinds: crate::core::usque::InstalledKinds) -> &'static str {
        match (kinds.wireguard, kinds.usque) {
            (true, true) => "warp.installed_both",
            (true, false) => "warp.installed_wireguard",
            (false, true) => "warp.installed_usque",
            // 都没有——这个分支只会在判定与显示不一致时命中，
            // 调用方应先拦在 `is_installed()` 之前。
            (false, false) => "warp.not_installed",
        }
    }

    /// WireGuard 单套的状态行（子页用）。
    ///
    /// 与 [`Self::status`] 的区别：`status` 同时讲两套，本函数只讲
    /// WireGuard——子页里混进 usque 的说明正是要消除的混淆。
    pub async fn wireguard_status_line() -> String {
        let registered = fs::try_exists(warp_paths::ACCOUNT_FILE)
            .await
            .unwrap_or(false);
        if !registered {
            return "WireGuard: 🔴 未注册".to_string();
        }

        let first = read_wg_rx_bytes().await;
        tokio::time::sleep(WG_RX_SAMPLE_INTERVAL).await;
        let second = read_wg_rx_bytes().await;

        match Self::verdict_from_samples(first, second) {
            WgTunnelHealth::Established => {
                format!("WireGuard: 🟢 隧道正常（近 {WG_RX_SAMPLE_INTERVAL:?} 内收到回包）")
            }
            WgTunnelHealth::NoTraffic => format!(
                "WireGuard: 🟡 已注册，但隧道未建立\n\
                 └ {WG_RX_SAMPLE_INTERVAL:?} 内 `wg0` 未收到任何回包"
            ),
        }
    }

    /// WARP 状态：账号是否注册 **且隧道是否真的通**。
    ///
    /// 修复前这里只 `try_exists(warp_account.json)`，于是**永远**显示
    /// 「🟢 已注册」——真机上 `wg0` 的 `rx_bytes` 恒为 0 持续数周，
    /// TG 里的状态检测始终是绿的。这个假阳性正是故障潜伏至今的根因：
    /// 没人能从 bot 侧看出隧道其实一个回包都收不到。
    ///
    /// 现改为三态：文件缺失 / 有文件但收不到回包 / 有文件且有回包。
    /// usque（MASQUE over TCP/443）的状态行。
    ///
    /// 纯函数，便于穷举测试而不用碰真实文件系统/网络。
    ///
    /// 与 WireGuard 那行**并列**显示：两套实现是并存的，
    /// 管理员需要同时看到两边的情况才能判断当前走的是哪条路。
    ///
    /// `installed == false` 不是错误——用户可能只装了其中一套。
    pub fn usque_status_line(installed: bool, healthy: Option<bool>) -> String {
        if !installed {
            return "⚡ <b>usque</b>: ⚪ 未安装（MASQUE/HTTP-2 不可用）".to_string();
        }
        match healthy {
            Some(true) => "⚡ <b>usque</b>: 🟢 隧道正常（warp=on）".to_string(),
            Some(false) => "⚡ <b>usque</b>: 🟡 已安装，但隧道未转发流量".to_string(),
            // 未探测——不谎报健康，也不谎报故障。
            None => "⚡ <b>usque</b>: ⚪ 已安装（未探测）".to_string(),
        }
    }

    pub async fn status() -> Result<String> {
        let is_registered = fs::try_exists(warp_paths::ACCOUNT_FILE)
            .await
            .unwrap_or(false);

        // usque 是否安装——与 WireGuard 独立判定。
        let usque_installed = crate::core::usque::installer::detect_installed()
            .await
            .usque;
        // 读后台健康循环维护的状态；**不**在渲染时探测（会阻塞最多 20s）。
        // 未探测过时是 `None` → 显示「未探测」，不谎报健康或故障。
        let usque_healthy = crate::core::usque::health::current_health();
        let usque_line = Self::usque_status_line(usque_installed, usque_healthy);

        if !is_registered {
            // WireGuard 没装不代表整体不可用——usque 可能装着呢。
            return Ok(format!(
                "⚙️ <b>WARP 状态</b>\n\
                 WireGuard: 🔴 未注册\n\
                 {usque_line}\n\
                 (Native Mode 由核心服务托管)"
            ));
        }

        // 采样两次 `rx_bytes`，用「是否有增长」而非「是否 > 0」判定。
        //
        // 单纯 `rx_bytes > 0` 无法区分「正在工作」与「曾经工作过、现在断了」：
        // 计数器是累计值，一旦恢复过一次就永久为正。那种情况下旧逻辑会
        // 再次发出假阳性绿报——而本部署恰好就是因为「恒为 0」才暴露了这个坑。
        let first = read_wg_rx_bytes().await;
        tokio::time::sleep(WG_RX_SAMPLE_INTERVAL).await;
        let second = read_wg_rx_bytes().await;

        let verdict = Self::verdict_from_samples(first, second);

        let wg_line = match verdict {
            WgTunnelHealth::Established => {
                format!("WireGuard: 🟢 隧道正常（近 {WG_RX_SAMPLE_INTERVAL:?} 内收到回包）")
            }
            WgTunnelHealth::NoTraffic => format!(
                "WireGuard: 🟡 已注册，但隧道未建立\n\
                 └ {WG_RX_SAMPLE_INTERVAL:?} 内 `wg0` 未收到任何回包"
            ),
        };

        // 两套都不可用时给出明确结论（对应 OQ3：不假装可用）。
        let unavailable_hint = if verdict == WgTunnelHealth::NoTraffic && !usque_installed {
            "\n\n⚠️ <b>两条 WARP 通路都不可用</b>：\n\
             走 UDP 的 WireGuard 收不到回包（机房常丢出站 UDP），\n\
             而走 TCP/443 的 usque 未安装。\n\
             请先用「出口连通性探测」确认出口情况。"
        } else {
            ""
        };

        Ok(format!(
            "⚙️ <b>WARP 状态</b>\n\
             {wg_line}\n\
             {usque_line}\n\
             (Native Mode 由核心服务托管){unavailable_hint}"
        ))
    }
}

#[allow(dead_code)]
struct ProgressState {
    running: bool,
    step: u8,
    total: u8,
    description: String,
}

impl ProgressState {
    fn new() -> Self {
        Self {
            running: false,
            step: 0,
            total: TOTAL_STEPS,
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealityInstallOutcome {
    AlreadyReady,
    Completed,
    InProgress,
}

#[allow(dead_code)]
pub struct RealityInstaller {
    msg_id: Option<AegisMsgId>,
    progress_state: Arc<Mutex<ProgressState>>,
}

impl RealityInstaller {
    pub async fn run(
        adapter: &dyn BotAdapter,
        target: &TargetId,
        msg_id: Option<&AegisMsgId>,
    ) -> Result<RealityInstallOutcome> {
        if MaintenanceManager::is_reality_base_ready().await {
            return Ok(RealityInstallOutcome::AlreadyReady);
        }

        let progress_state = Arc::new(Mutex::new(ProgressState::new()));

        {
            let mut state = progress_state.lock().await;
            state.running = true;
        }

        let installer = RealityInstallerInternal {
            adapter,
            target,
            msg_id: msg_id.cloned(),
            progress_state: progress_state.clone(),
        };

        let outcome = match installer.execute().await {
            Ok(_) => RealityInstallOutcome::Completed,
            Err(err) => {
                installer.report_failure(&err).await;
                {
                    let mut state = progress_state.lock().await;
                    state.running = false;
                }
                return Err(err);
            }
        };

        {
            let mut state = progress_state.lock().await;
            state.running = false;
            state.step = TOTAL_STEPS;
            state.description = "✅ 初始化完成".to_string();
        }

        Ok(outcome)
    }
}

pub struct RealityInstallerInternal<'a> {
    adapter: &'a dyn BotAdapter,
    target: &'a TargetId,
    msg_id: Option<AegisMsgId>,
    progress_state: Arc<Mutex<ProgressState>>,
}

impl<'a> RealityInstallerInternal<'a> {
    /// This is a legacy static entry point. For progress updates, use the `run` method.
    pub async fn install_minimal_environment() -> Result<()> {
        let probe = SystemProbe::detect().await.context("系统检测失败")?;
        // For static calls without Bot context, we just run it without progress updates.
        ensure_directories().await?;
        Self::install_toolchain_static(&probe.package_manager)
            .await
            .context("安装依赖失败")?;
        Self::step_install_core(probe.arch).await?;
        Self::step_configure_service().await?;
        if let Err(e) = crate::core::xray::config::ConfigManager::ensure_base_config().await {
            log::warn!("创建 Xray-core 基础配置失败: {}", e);
        }
        if let Err(e) = crate::core::system::maintenance::MaintenanceManager::ensure_geodata().await
        {
            log::warn!("确保 geodata 文件失败: {}", e);
        }
        let _ = MaintenanceManager::reload_core().await;
        Ok(())
    }

    pub async fn install_toolchain_static(manager: &PackageManager) -> Result<()> {
        manager.update().await.with_context(|| "更新软件仓库失败")?;
        let packages = match manager {
            PackageManager::Apt => vec![
                "sudo",
                "curl",
                "wget",
                "jq",
                "tar",
                "unzip",
                "net-tools",
                "cron",
                "ca-certificates",
                "lsb-release",
                "chrony",
            ],
            PackageManager::Yum => vec![
                "sudo",
                "curl",
                "wget",
                "jq",
                "tar",
                "unzip",
                "net-tools",
                "cronie",
                "ca-certificates",
                "chrony",
            ],
            PackageManager::Apk => BASE_PACKAGES_APK.to_vec(),
        };

        let mut missing_packages = Vec::new();
        for pkg in packages {
            if !manager.check_installed(pkg).await {
                missing_packages.push(pkg);
            }
        }

        if !missing_packages.is_empty() {
            manager
                .install(&missing_packages)
                .await
                .with_context(|| format!("安装依赖 {:?} 失败", missing_packages))?;
        }

        if crate::core::security::firewall::FirewallManager::detect_backend()
            .await
            .is_none()
            && !manager.check_installed("firewalld").await
        {
            manager
                .install(&["firewalld"])
                .await
                .with_context(|| "安装 firewalld 失败")?;
        }
        Ok(())
    }

    async fn execute(&self) -> Result<()> {
        self.update_progress(0, "准备初始化环境").await?;
        let probe = SystemProbe::detect().await.context("系统检测失败")?;

        self.update_progress(1, "安装基础依赖").await?;
        self.step_install_dependencies(&probe).await?;

        self.update_progress(2, "安装 Xray-core 核心").await?;
        Self::step_install_core(probe.arch).await?;

        self.update_progress(3, "配置 Xray-core 服务").await?;
        Self::step_configure_service().await?;

        self.update_progress(4, "验证服务状态").await?;
        MaintenanceManager::reload_core()
            .await
            .context("服务启动失败")?;

        self.update_progress(5, "启用防火墙服务").await?;
        Self::step_enable_firewall().await?;

        self.update_progress(TOTAL_STEPS, "✅ 初始化完成").await?;
        Ok(())
    }

    pub async fn step_install_dependencies(&self, probe: &SystemProbe) -> Result<()> {
        ensure_directories().await?;
        self.install_toolchain_with_logs(&probe.package_manager)
            .await
            .context("安装依赖失败")
    }

    pub async fn step_install_core(arch: CpuArch) -> Result<()> {
        install_wwps_core(arch).await
    }

    pub async fn step_configure_service() -> Result<()> {
        install_wwps_core_service()
            .await
            .context("配置 Xray-core 服务失败")
    }

    pub async fn step_enable_firewall() -> Result<()> {
        if let Some(backend) =
            crate::core::security::firewall::FirewallManager::detect_backend().await
        {
            match backend {
                crate::core::security::firewall::FirewallBackend::Ufw => {
                    let _ = run_command("ufw", &["--force", "enable"]).await;
                }
                crate::core::security::firewall::FirewallBackend::Firewalld => {
                    if is_systemd().await {
                        let _ = run_command("systemctl", &["enable", "--now", "firewalld"]).await;
                    } else if is_openrc().await {
                        let _ = run_command("rc-update", &["add", "firewalld", "default"]).await;
                        let _ = run_command("rc-service", &["firewalld", "start"]).await;
                    }
                }
            }
        }
        Ok(())
    }

    async fn update_progress(&self, step: u8, desc: &str) -> Result<()> {
        {
            let mut state = self.progress_state.lock().await;
            state.step = step.min(TOTAL_STEPS);
            state.description = desc.to_string();
        }
        let text = build_progress_text(step.min(TOTAL_STEPS), TOTAL_STEPS, desc, false);
        if let Some(msg_id) = &self.msg_id {
            let _ = self
                .adapter
                .edit_message(self.target, msg_id, MessageContent { text, markup: None })
                .await;
        }
        Ok(())
    }

    pub async fn install_toolchain_with_logs(&self, manager: &PackageManager) -> Result<()> {
        self.update_progress(1, "正在更新软件仓库...").await?;
        manager.update().await.with_context(|| "更新软件仓库失败")?;

        let packages = match manager {
            PackageManager::Apt => vec![
                "sudo",
                "curl",
                "wget",
                "jq",
                "tar",
                "unzip",
                "net-tools",
                "cron",
                "ca-certificates",
                "lsb-release",
                "chrony",
            ],
            PackageManager::Yum => vec![
                "sudo",
                "curl",
                "wget",
                "jq",
                "tar",
                "unzip",
                "net-tools",
                "cronie",
                "ca-certificates",
                "chrony",
            ],
            PackageManager::Apk => BASE_PACKAGES_APK.to_vec(),
        };

        for (i, pkg) in packages.iter().enumerate() {
            let desc = format!("正在检查依赖 ({}/{}): {}", i + 1, packages.len(), pkg);
            self.update_progress(1, &desc).await?; // Stay on step 1

            if !manager.check_installed(pkg).await {
                let desc = format!("正在安装依赖 ({}/{}): {}", i + 1, packages.len(), pkg);
                self.update_progress(1, &desc).await?;
                manager
                    .install(&[pkg])
                    .await
                    .with_context(|| format!("安装 {} 失败", pkg))?;
            }
        }
        if crate::core::security::firewall::FirewallManager::detect_backend()
            .await
            .is_none()
        {
            self.update_progress(1, "正在检查防火墙组件...").await?;
            if !manager.check_installed("firewalld").await {
                self.update_progress(1, "正在安装 firewalld...").await?;
                manager
                    .install(&["firewalld"])
                    .await
                    .with_context(|| "安装 firewalld 失败")?;
            }
        }
        Ok(())
    }

    async fn report_failure(&self, err: &anyhow::Error) {
        let text = format!(
            "❌ <b>Reality 初始化失败</b>\n\n原因: {}\n\n请检查系统环境或尝试 install.sh 回退流程。",
            err
        );
        if let Some(msg_id) = &self.msg_id {
            let _ = self
                .adapter
                .edit_message(self.target, msg_id, MessageContent { text, markup: None })
                .await;
        }
    }
}

pub struct SystemProbe {
    pub package_manager: PackageManager,
    pub arch: CpuArch,
}

impl SystemProbe {
    pub async fn detect() -> Result<Self> {
        let package_manager = PackageManager::detect().await?;
        let arch = CpuArch::detect().context("检测 CPU 架构失败")?;
        Ok(Self {
            package_manager,
            arch,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PackageManager {
    Apt,
    Yum,
    Apk,
}

impl PackageManager {
    pub(crate) async fn detect() -> Result<Self> {
        if fs::try_exists("/etc/debian_version").await.unwrap_or(false) {
            Ok(Self::Apt)
        } else if fs::try_exists("/etc/alpine-release").await.unwrap_or(false) {
            Ok(Self::Apk)
        } else if fs::try_exists("/etc/redhat-release").await.unwrap_or(false)
            || fs::try_exists("/etc/centos-release").await.unwrap_or(false)
        {
            Ok(Self::Yum)
        } else {
            Err(anyhow!("无法识别当前系统的包管理器"))
        }
    }

    pub(crate) async fn update(&self) -> Result<()> {
        match self {
            PackageManager::Apt => run_command("apt-get", &["update"]).await,
            PackageManager::Yum => run_command("yum", &["makecache"]).await,
            PackageManager::Apk => run_command("apk", &["update"]).await,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn upgrade_all(&self) -> Result<()> {
        match self {
            PackageManager::Apt => run_command("apt-get", &["upgrade", "-y"]).await,
            PackageManager::Yum => run_command("yum", &["update", "-y"]).await,
            PackageManager::Apk => run_command("apk", &["upgrade", "--no-progress"]).await,
        }
    }

    async fn install(&self, packages: &[&str]) -> Result<()> {
        match self {
            PackageManager::Apt => {
                let mut args = vec!["install", "-y"];
                args.extend_from_slice(packages);
                run_command("apt-get", &args).await
            }
            PackageManager::Yum => {
                let mut args = vec!["install", "-y"];
                args.extend_from_slice(packages);
                run_command("yum", &args).await
            }
            PackageManager::Apk => {
                let mut args = vec!["add", "--no-progress"];
                args.extend_from_slice(packages);
                run_command("apk", &args).await
            }
        }
    }

    async fn check_installed(&self, pkg: &str) -> bool {
        match self {
            PackageManager::Apt => {
                // dpkg -s <pkg>
                Command::new("dpkg")
                    .args(["-s", pkg])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
            PackageManager::Yum => {
                // rpm -q <pkg>
                Command::new("rpm")
                    .args(["-q", pkg])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
            PackageManager::Apk => {
                // apk info -e <pkg>
                Command::new("apk")
                    .args(["info", "-e", pkg])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
        }
    }
}

pub async fn install_wwps_core(arch: CpuArch) -> Result<()> {
    let install_dir = PathBuf::from(xray::DIR);
    let backup_dir = PathBuf::from(xray::BACKUP_DIR);
    let temp_dir = PathBuf::from(WWPS_CORE_TEMP_DIR);

    fs::create_dir_all(&install_dir)
        .await
        .context("创建 Xray-core 安装目录失败")?;
    fs::create_dir_all(&backup_dir)
        .await
        .context("创建 Xray-core 备份目录失败")?;
    fs::create_dir_all(&temp_dir)
        .await
        .context("创建 Xray-core 临时目录失败")?;

    let config = WwpsCoreUpgradeConfig::new(
        "XTLS",
        "Xray-core",
        "wwps-core",
        install_dir.clone(),
        backup_dir,
        temp_dir.clone(),
        arch,
    );
    let manager = WwpsCoreUpgradeManager::new(config).context("构建 Xray-core 升级管理器失败")?;
    let release = manager.fetch_release(None).await?;
    let archive = manager.download_release(&release, None, None, None).await?;
    let unpack = manager.extract_archive(&archive).await?;
    manager.replace_core(&unpack).await?;
    manager
        .cleanup_paths(&[archive.clone(), unpack.clone()])
        .await;
    Ok(())
}

pub async fn install_wwps_core_service() -> Result<()> {
    if is_systemd().await {
        install_systemd_service().await
    } else if is_openrc().await {
        install_openrc_service().await
    } else {
        Err(anyhow!("未检测到受支持的服务管理器 (systemd/openrc)"))
    }
}

pub async fn ensure_directories() -> Result<()> {
    const DIRS: &[&str] = &[
        "/etc/wwps",
        xray::DIR,
        xray::CONF_DIR,
        xray::TEMP_DIR,
        "/etc/wwps/subscribe",
        "/etc/wwps/subscribe/default",
        "/etc/wwps/subscribe_local/default",
    ];

    for dir in DIRS {
        fs::create_dir_all(dir)
            .await
            .with_context(|| format!("创建目录 {} 失败", dir))?;
    }
    Ok(())
}

async fn install_systemd_service() -> Result<()> {
    const SERVICE_PATH: &str = "/etc/systemd/system/wwps-core.service";
    let unit = format!(
        "[Unit]\nDescription=wwps-core Service\nAfter=network.target\n\n[Service]\nUser=root\nType=simple\nExecStart={}/wwps-core run -confdir {}/conf\nRestart=always\nRestartSec=5\nLimitNOFILE=51200\n\n[Install]\nWantedBy=multi-user.target\n",
        xray::DIR,
        xray::DIR
    );

    fs::write(SERVICE_PATH, unit)
        .await
        .context("写入 systemd 服务文件失败")?;

    run_command("systemctl", &["daemon-reload"]).await?;
    run_command("systemctl", &["enable", "--now", "wwps-core.service"]).await?;
    Ok(())
}

async fn install_openrc_service() -> Result<()> {
    const SERVICE_PATH: &str = "/etc/init.d/wwps-core";
    let script = format!(
        "#!/sbin/openrc-run\ndescription=\"wwps-core Service\"\ncommand=\"{}/wwps-core\"\ncommand_args=\"run -confdir {}/conf\"\npidfile=/run/wwps-core.pid\ncommand_background=yes\ndepend() {{\n    need net\n}}\n",
        xray::DIR,
        xray::DIR
    );

    fs::write(SERVICE_PATH, script)
        .await
        .context("写入 OpenRC 脚本失败")?;
    let mut perms = fs::metadata(SERVICE_PATH).await?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(SERVICE_PATH, perms).await?;

    run_command("rc-update", &["add", "wwps-core", "default"]).await?;
    run_command("rc-service", &["wwps-core", "restart"]).await?;
    Ok(())
}

fn build_progress_text(step: u8, total: u8, desc: &str, existing: bool) -> String {
    let bar = progress_bar(step, total);
    let heading = if existing {
        "🚧 <b>Reality 初始化进行中</b>"
    } else {
        "🚀 <b>Reality 初始化</b>"
    };
    format!(
        "{}\n{}\n\n📍 {}\n\n🔐 本次将自动通过 wwps-core/xray (Xray-core) mldsa65 生成 Reality PQ (ML-DSA-65) seed/verify，与 Xray 完全兼容。\n\n请勿关闭窗口，完成后将自动返回批量界面。",
        heading, bar, desc
    )
}

fn progress_bar(step: u8, total: u8) -> String {
    if total == 0 {
        return "[░░░░░░░░░░] 0%".to_string();
    }
    let segments = 10;
    let ratio = step as f32 / total as f32;
    let filled = (ratio * segments as f32).round() as usize;
    let filled = filled.min(segments);
    let mut bar = String::from("[");
    for i in 0..segments {
        if i < filled {
            bar.push('▓');
        } else {
            bar.push('░');
        }
    }
    bar.push(']');
    let percent = (ratio * 100.0).round() as i32;
    format!("{} {}%", bar, percent.clamp(0, 100))
}

async fn run_command(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .await
        .with_context(|| format!("执行命令 {} {:?} 失败", cmd, args))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("命令 {} {:?} 返回非零状态", cmd, args))
    }
}

async fn is_systemd() -> bool {
    fs::try_exists("/run/systemd/system").await.unwrap_or(false)
        || fs::try_exists("/bin/systemctl").await.unwrap_or(false)
}

async fn is_openrc() -> bool {
    fs::try_exists("/run/openrc").await.unwrap_or(false)
        || fs::try_exists("/sbin/openrc").await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reality_install_outcome_variants() {
        assert_eq!(
            RealityInstallOutcome::AlreadyReady,
            RealityInstallOutcome::AlreadyReady
        );
        assert_eq!(
            RealityInstallOutcome::Completed,
            RealityInstallOutcome::Completed
        );
        assert_eq!(
            RealityInstallOutcome::InProgress,
            RealityInstallOutcome::InProgress
        );
    }

    #[test]
    fn test_reality_install_outcome_not_equal() {
        assert_ne!(
            RealityInstallOutcome::AlreadyReady,
            RealityInstallOutcome::Completed
        );
        assert_ne!(
            RealityInstallOutcome::Completed,
            RealityInstallOutcome::InProgress
        );
    }

    #[test]
    fn test_reality_install_outcome_debug() {
        assert!(format!("{:?}", RealityInstallOutcome::AlreadyReady).contains("AlreadyReady"));
        assert!(format!("{:?}", RealityInstallOutcome::Completed).contains("Completed"));
    }

    #[test]
    fn test_progress_state_new() {
        let state = ProgressState::new();
        assert!(!state.running);
        assert_eq!(state.step, 0);
        assert_eq!(state.total, TOTAL_STEPS);
        assert!(state.description.is_empty());
    }

    #[test]
    fn test_progress_state_total_steps() {
        assert_eq!(TOTAL_STEPS, 6);
    }

    // ───────────────────── T10: 隧道健康判定 ─────────────────────

    /// 收到回包（计数增长）→ 已建立。
    #[test]
    fn test_verdict_established_when_rx_grows() {
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(0), Some(1024)),
            WgTunnelHealth::Established
        );
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(4096), Some(8192)),
            WgTunnelHealth::Established
        );
    }

    /// **Bug B 的核心回归测试**：计数为 0（真机现状）必须是「未建立」。
    ///
    /// 修复前这里会报「🟢 已注册」，正是该假阳性让故障潜伏数周。
    #[test]
    fn test_verdict_not_established_when_rx_is_zero() {
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(0), Some(0)),
            WgTunnelHealth::NoTraffic
        );
    }

    /// 计数为正但**无增长** → 仍未建立。
    ///
    /// 这防的是「曾经通过、现在断了」：计数器是累计值，一旦通过一次
    /// 就永久为正，用 `> 0` 判定会再次发出假阳性。
    #[test]
    fn test_verdict_not_established_when_rx_stalls() {
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(1_000_000), Some(1_000_000)),
            WgTunnelHealth::NoTraffic,
            "累计值不增长必须判为未建立，否则「曾经通、现在断」会再次假绿"
        );
    }

    /// 读不到计数器（接口未建 / 路径不存在）→ 未建立，且不 panic。
    #[test]
    fn test_verdict_handles_unreadable_counter() {
        assert_eq!(
            WarpInstaller::verdict_from_samples(None, None),
            WgTunnelHealth::NoTraffic
        );
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(100), None),
            WgTunnelHealth::NoTraffic
        );
        assert_eq!(
            WarpInstaller::verdict_from_samples(None, Some(100)),
            WgTunnelHealth::NoTraffic
        );
    }

    /// 计数器回绕（理论可能）不得误判为已建立。
    #[test]
    fn test_verdict_does_not_treat_wrap_as_established() {
        assert_eq!(
            WarpInstaller::verdict_from_samples(Some(u64::MAX), Some(1)),
            WgTunnelHealth::NoTraffic,
            "回绕后 second < first，不应判为已建立"
        );
    }

    // ───────────────────── T9: 重装保留配置 ─────────────────────

    /// 首次安装（无既有配置）→ 默认：空清单 + Default 模式。
    #[test]
    fn test_restore_defaults_on_first_install() {
        let (rules, mode) =
            WarpInstaller::config_to_restore(None, crate::core::xray::WarpScope::Domain);
        assert!(rules.is_empty(), "首次安装不应有域名规则");
        assert_eq!(mode, crate::core::xray::WarpMode::Default);
    }

    /// **Bug A 的核心回归测试**：重装必须原样保留管理员配的清单与模式。
    ///
    /// 修复前 `install()` 无条件传 `vec![]` + `Default`，每次重装都
    /// 静默清空域名规则并重置出口协议偏好（真机已复现）。
    #[test]
    fn test_restore_preserves_existing_config_on_reinstall() {
        let saved = vec![
            "domain:reddit.com".to_string(),
            "domain:example.org".to_string(),
        ];

        for mode in [
            crate::core::xray::WarpMode::Default,
            crate::core::xray::WarpMode::IPv4,
            crate::core::xray::WarpMode::IPv6,
        ] {
            let (rules, restored_mode) = WarpInstaller::config_to_restore(
                Some((saved.clone(), mode)),
                crate::core::xray::WarpScope::Domain,
            );
            assert_eq!(rules, saved, "重装后域名清单必须逐项保留（mode={mode:?}）");
            assert_eq!(restored_mode, mode, "重装后模式偏好必须保留");
        }
    }

    /// scope 不影响保留逻辑：它不落在路由文件里，由调用方单独处理。
    #[test]
    fn test_restore_is_independent_of_scope() {
        let saved = vec!["domain:a.com".to_string()];
        let a = WarpInstaller::config_to_restore(
            Some((saved.clone(), crate::core::xray::WarpMode::IPv4)),
            crate::core::xray::WarpScope::Domain,
        );
        let b = WarpInstaller::config_to_restore(
            Some((saved, crate::core::xray::WarpMode::IPv4)),
            crate::core::xray::WarpScope::Global,
        );
        assert_eq!(a, b, "scope 不应改变保留结果");
    }

    // ───────────────── T14: usque 状态行 ─────────────────

    /// `usque_status_line` 穷举 4 种输入组合。
    ///
    /// 关键：**未安装不是错误**（用户可能只装了一套），
    /// 且未探测时不得谎报健康或故障。
    #[test]
    fn test_usque_status_line_all_states() {
        let not_installed = WarpInstaller::usque_status_line(false, None);
        assert!(not_installed.contains("未安装"), "{not_installed}");

        let installed_healthy = WarpInstaller::usque_status_line(true, Some(true));
        assert!(installed_healthy.contains("正常"), "{installed_healthy}");

        let installed_sick = WarpInstaller::usque_status_line(true, Some(false));
        assert!(
            installed_sick.contains("未转发"),
            "已装但不健康必须明说: {installed_sick}"
        );

        let installed_unknown = WarpInstaller::usque_status_line(true, None);
        assert!(
            installed_unknown.contains("未探测"),
            "未探测时不得谎报健康/故障: {installed_unknown}"
        );
    }

    /// 「已装但不健康」与「已装且健康」必须产生**不同**文案——
    /// 否则状态页看不出区别，又回到假阳性老路。
    #[test]
    fn test_usque_healthy_and_sick_differ() {
        let h = WarpInstaller::usque_status_line(true, Some(true));
        let s = WarpInstaller::usque_status_line(true, Some(false));
        assert_ne!(h, s, "健康与不健康必须能区分");
    }

    /// 未安装时**不得**出现「正常」字样——否则会让人以为 MASQUE 可用。
    #[test]
    fn test_usque_not_installed_never_claims_healthy() {
        let s = WarpInstaller::usque_status_line(false, None);
        assert!(!s.contains("正常"), "未安装不得显示正常: {s}");
        assert!(!s.contains("🟢"), "未安装不得显示绿灯: {s}");
    }

    // ───────── T2: 写缓存 → 出口引擎随之变化 ─────────

    /// **核心因果关系**：写入探测结果后，出口引擎必须立刻反映它。
    ///
    /// 这条测试存在的理由：缓存此前**没有任何写入点**，于是
    /// `current_egress()` 永远回退 `Direct`，装了 usque 也不生效。
    /// 若后人把 `record_probe_result` 的调用删掉，这条会失败。
    #[test]
    fn test_record_probe_result_drives_egress_selection() {
        use crate::core::network::{ProbeConclusion, ProbeReport, TcpVerdict, UdpVerdict};

        let cache = crate::core::usque::engine::global_probe_cache();

        // 先清空，确认「未探测 → Direct」
        cache.clear();
        assert_eq!(
            crate::core::xray::warp::test_egress_now(),
            crate::core::usque::Egress::Direct,
            "无缓存时应回退 Direct"
        );

        // 本部署的真实处境：UDP 被封、TCP/443 可用
        WarpInstaller::record_probe_result(ProbeReport {
            udp: UdpVerdict::Blocked,
            tcp: TcpVerdict::AvailableH2,
            conclusion: ProbeConclusion::TcpOnly,
        });

        assert_eq!(
            crate::core::xray::warp::test_egress_now(),
            crate::core::usque::Egress::Masque,
            "写入 TcpOnly 后出口引擎必须选 MASQUE（这正是「安装后能生效」的机械保证）"
        );

        cache.clear();
    }

    /// 写入 `WgUsable` 则应选 WireGuard。
    #[test]
    fn test_record_probe_result_selects_wireguard_when_udp_works() {
        use crate::core::network::{ProbeConclusion, ProbeReport, TcpVerdict, UdpVerdict};

        WarpInstaller::record_probe_result(ProbeReport {
            udp: UdpVerdict::Available,
            tcp: TcpVerdict::AvailableH2,
            conclusion: ProbeConclusion::WgUsable,
        });
        assert_eq!(
            crate::core::xray::warp::test_egress_now(),
            crate::core::usque::Egress::WireGuard
        );
        crate::core::usque::engine::global_probe_cache().clear();
    }

    // ────────── 安装判定必须覆盖两套实现 ──────────

    /// **防回归**：`is_installed()` 不得只看 WireGuard。
    ///
    /// # 已发生的故障
    ///
    /// 线上只装了 usque（WireGuard 未注册），而 `is_installed()` 只
    /// `try_exists(ACCOUNT_FILE)` → 返回 `false` → 菜单显示
    /// 「⚠️ 未检测到 Cloudflare WARP」并给出安装按钮。
    ///
    /// 管理员此时会**反复点安装**：每次都成功（usque 已装会跳过），
    /// 但界面永远说没装。这是一个无法自解的死循环，且看起来像
    /// 「安装失败」而不是「判定写错了」。
    ///
    /// 断言源码而非运行时：真实判定要读磁盘，而这里要钉死的是
    /// 「有没有考虑第二套实现」这一结构性质。
    #[test]
    fn test_is_installed_must_consider_both_implementations() {
        const SRC: &str = include_str!("installer.rs");
        let start = SRC
            .find("pub async fn is_installed()")
            .expect("is_installed 必须存在");
        let body = &SRC[start..];
        let body = &body[..body.find("\n    }").unwrap_or(body.len())];

        assert!(
            body.contains("detect_installed"),
            "is_installed() 必须基于 detect_installed() 判定两套实现。\n\
             只查 warp_paths::ACCOUNT_FILE 会让「只装 usque」的机房\n\
             永远显示未安装——线上已发生过这个死循环故障。"
        );
        assert!(
            !body.contains("warp_paths::ACCOUNT_FILE"),
            "is_installed() 不得直接检查 WireGuard 的 ACCOUNT_FILE——\n\
             usque 的账号文件在 /etc/wwps/usque/config.json，是另一个路径。"
        );
    }

    /// 判定结果要能区分「装了哪一套」，供界面分派显示。
    #[test]
    fn test_install_kind_is_distinguishable() {
        use crate::core::usque::InstalledKinds;

        let wg_only = InstalledKinds::detect(true, false);
        let uq_only = InstalledKinds::detect(false, true);
        let both = InstalledKinds::detect(true, true);
        let none = InstalledKinds::none();

        // 都算「已安装」——入口不应再拦。
        assert!(wg_only.any(), "只装 WireGuard 也算已安装");
        assert!(uq_only.any(), "只装 usque 也算已安装（这正是线上情形）");
        assert!(both.any());
        assert!(!none.any(), "都没装才是未安装");

        // 但必须能区分，否则界面无法显示「装的是哪一套」。
        assert_ne!(wg_only, uq_only, "两套必须可区分，界面要据此分派文案");
        assert_ne!(uq_only, none);
    }

    /// 横幅文案必须能区分四种组合——穷举，因为漏掉任一种都会让
    /// 管理员看到与实际不符的描述。
    #[test]
    fn test_installed_banner_key_dispatches_all_cases() {
        use crate::core::usque::InstalledKinds;

        let cases = [
            (InstalledKinds::detect(true, true), "warp.installed_both"),
            (
                InstalledKinds::detect(true, false),
                "warp.installed_wireguard",
            ),
            (InstalledKinds::detect(false, true), "warp.installed_usque"),
            (InstalledKinds::none(), "warp.not_installed"),
        ];
        for (kinds, expect) in cases {
            assert_eq!(
                WarpInstaller::installed_banner_key(kinds),
                expect,
                "wireguard={} usque={} 应显示 {expect}",
                kinds.wireguard,
                kinds.usque
            );
        }
    }

    /// **线上故障的直接回归**：只装 usque 时，横幅必须是 usque 那一条，
    /// 不能是「未安装」。
    #[test]
    fn test_usque_only_is_not_reported_as_not_installed() {
        use crate::core::usque::InstalledKinds;

        let key = WarpInstaller::installed_banner_key(InstalledKinds::detect(false, true));
        assert_eq!(key, "warp.installed_usque");
        assert_ne!(
            key, "warp.not_installed",
            "只装 usque 就报「未检测到 WARP」——这正是线上那个死循环"
        );
    }
}
