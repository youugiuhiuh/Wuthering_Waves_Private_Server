use crate::logic::maintenance::MaintenanceManager;
use crate::logic::upgrade::wwps_core::{CpuArch, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId, ParseMode};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Mutex;

const TOTAL_STEPS: u8 = 6;
const WWPS_CORE_INSTALL_DIR: &str = "/etc/wwps/wwps-core";
const WWPS_CORE_BACKUP_DIR: &str = "/etc/wwps/wwps-core/backup";
const WWPS_CORE_TEMP_DIR: &str = "/tmp/wwps-core-installer";

static PROGRESS_STATE: Lazy<Mutex<ProgressState>> = Lazy::new(|| {
    Mutex::new(ProgressState {
        running: false,
        step: 0,
        total: TOTAL_STEPS,
        description: String::new(),
    })
});

struct ProgressState {
    running: bool,
    step: u8,
    total: u8,
    description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealityInstallOutcome {
    AlreadyReady,
    Completed,
    InProgress,
}

pub struct RealityInstaller;

impl RealityInstaller {
    pub async fn run(
        bot: Bot,
        chat_id: ChatId,
        msg_id: MessageId,
    ) -> Result<RealityInstallOutcome> {
        if MaintenanceManager::is_reality_base_ready().await {
            return Ok(RealityInstallOutcome::AlreadyReady);
        }

        {
            let mut state = PROGRESS_STATE.lock().await;
            if state.running {
                let text = build_progress_text(state.step, state.total, &state.description, true);
                drop(state);
                let _ = bot
                    .edit_message_text(chat_id, msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .await;
                return Ok(RealityInstallOutcome::InProgress);
            }
            state.running = true;
            state.step = 0;
            state.total = TOTAL_STEPS;
            state.description.clear();
        }

        let installer = RealityInstallerInternal {
            bot: bot.clone(),
            chat_id,
            msg_id,
        };

        let outcome = match installer.execute().await {
            Ok(_) => RealityInstallOutcome::Completed,
            Err(err) => {
                installer.report_failure(&err).await;
                {
                    let mut state = PROGRESS_STATE.lock().await;
                    state.running = false;
                }
                return Err(err);
            }
        };

        {
            let mut state = PROGRESS_STATE.lock().await;
            state.running = false;
            state.step = TOTAL_STEPS;
            state.description = "✅ 初始化完成".to_string();
        }

        Ok(outcome)
    }
}

pub struct RealityInstallerInternal {
    bot: Bot,
    chat_id: ChatId,
    msg_id: MessageId,
}

impl RealityInstallerInternal {
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
            PackageManager::Apk => vec![
                "sudo", "curl", "wget", "jq", "tar", "unzip", "bash", "openrc", "chrony",
            ],
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

        if crate::logic::firewall::FirewallManager::detect_backend()
            .await
            .is_none()
        {
            if !manager.check_installed("firewalld").await {
                manager
                    .install(&["firewalld"])
                    .await
                    .with_context(|| "安装 firewalld 失败")?;
            }
        }
        Ok(())
    }

    async fn execute(&self) -> Result<()> {
        self.update_progress(0, "准备初始化环境").await?;
        let probe = SystemProbe::detect().await.context("系统检测失败")?;

        self.update_progress(1, "安装基础依赖").await?;
        self.step_install_dependencies(&probe).await?;

        self.update_progress(2, "安装 wwps-core 核心").await?;
        Self::step_install_core(probe.arch).await?;

        self.update_progress(3, "配置 wwps-core 服务").await?;
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
            .context("配置 wwps-core 服务失败")
    }

    pub async fn step_enable_firewall() -> Result<()> {
        if let Some(backend) = crate::logic::firewall::FirewallManager::detect_backend().await {
            match backend {
                crate::logic::firewall::FirewallBackend::Ufw => {
                    let _ = run_command("ufw", &["--force", "enable"]).await;
                }
                crate::logic::firewall::FirewallBackend::Firewalld => {
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
            let mut state = PROGRESS_STATE.lock().await;
            state.step = step.min(TOTAL_STEPS);
            state.description = desc.to_string();
        }
        let text = build_progress_text(step.min(TOTAL_STEPS), TOTAL_STEPS, desc, false);
        let _ = self
            .bot
            .edit_message_text(self.chat_id, self.msg_id, text)
            .parse_mode(ParseMode::Html)
            .await;
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
            PackageManager::Apk => vec![
                "sudo", "curl", "wget", "jq", "tar", "unzip", "bash", "openrc", "chrony",
            ],
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
        if crate::logic::firewall::FirewallManager::detect_backend()
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
        let _ = self
            .bot
            .edit_message_text(self.chat_id, self.msg_id, text)
            .parse_mode(ParseMode::Html)
            .await;
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
    async fn detect() -> Result<Self> {
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

    async fn update(&self) -> Result<()> {
        match self {
            PackageManager::Apt => run_command("apt-get", &["update"]).await,
            PackageManager::Yum => run_command("yum", &["makecache"]).await,
            PackageManager::Apk => run_command("apk", &["update"]).await,
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
                    .args(&["-s", pkg])
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
                    .args(&["-q", pkg])
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
                    .args(&["info", "-e", pkg])
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
    let install_dir = PathBuf::from(WWPS_CORE_INSTALL_DIR);
    let backup_dir = PathBuf::from(WWPS_CORE_BACKUP_DIR);
    let temp_dir = PathBuf::from(WWPS_CORE_TEMP_DIR);

    fs::create_dir_all(&install_dir)
        .await
        .context("创建 wwps-core 安装目录失败")?;
    fs::create_dir_all(&backup_dir)
        .await
        .context("创建 wwps-core 备份目录失败")?;
    fs::create_dir_all(&temp_dir)
        .await
        .context("创建 wwps-core 临时目录失败")?;

    let config = WwpsCoreUpgradeConfig::new(
        "XTLS",
        "Xray-core",
        "wwps-core",
        install_dir.clone(),
        backup_dir,
        temp_dir.clone(),
        arch,
    );
    let manager = WwpsCoreUpgradeManager::new(config).context("构建 wwps-core 升级管理器失败")?;
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
        "/etc/wwps/wwps-core",
        "/etc/wwps/wwps-core/conf",
        "/etc/wwps/wwps-core/tmp",
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
        WWPS_CORE_INSTALL_DIR, WWPS_CORE_INSTALL_DIR
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
        WWPS_CORE_INSTALL_DIR, WWPS_CORE_INSTALL_DIR
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
        "{}\n{}\n\n📍 {}\n\n请勿关闭窗口，完成后将自动返回批量界面。",
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
