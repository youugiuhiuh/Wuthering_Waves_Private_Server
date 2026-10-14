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
    pub async fn is_installed() -> bool {
        // Check for config file instead of binary
        fs::try_exists(warp_paths::ACCOUNT_FILE)
            .await
            .unwrap_or(false)
    }

    pub async fn install() -> Result<()> {
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

    pub async fn uninstall() -> Result<()> {
        // Just remove the account config and the routing config
        let _ = fs::remove_file(warp_paths::ACCOUNT_FILE).await;
        let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn restart_service() -> Result<()> {
        // No service to restart, just reload core
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    /// WARP 状态：账号是否注册 **且隧道是否真的通**。
    ///
    /// 修复前这里只 `try_exists(warp_account.json)`，于是**永远**显示
    /// 「🟢 已注册」——真机上 `wg0` 的 `rx_bytes` 恒为 0 持续数周，
    /// TG 里的状态检测始终是绿的。这个假阳性正是故障潜伏至今的根因：
    /// 没人能从 bot 侧看出隧道其实一个回包都收不到。
    ///
    /// 现改为三态：文件缺失 / 有文件但收不到回包 / 有文件且有回包。
    pub async fn status() -> Result<String> {
        let is_registered = fs::try_exists(warp_paths::ACCOUNT_FILE)
            .await
            .unwrap_or(false);

        if !is_registered {
            return Ok("⚙️ <b>账户状态</b>: 🔴 未注册\n(Native Mode 由核心服务托管)".to_string());
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

        Ok(match verdict {
            WgTunnelHealth::Established => format!(
                "⚙️ <b>账户状态</b>: 🟢 隧道正常（近 {WG_RX_SAMPLE_INTERVAL:?} 内收到回包）\n(Native Mode 由核心服务托管)"
            ),
            WgTunnelHealth::NoTraffic => format!(
                "⚙️ <b>账户状态</b>: 🟡 已注册，但隧道未建立\n\
                 已注册账号但 {WG_RX_SAMPLE_INTERVAL:?} 内 `wg0` 未收到任何回包。\n\
                 常见原因：机房在出站方向丢弃了 UDP 2408（本部署实测被封锁）。\n\
                 请用「出口连通性探测」确认，或改用 TCP/443（MASQUE）。"
            ),
        })
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
}
