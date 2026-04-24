use anyhow::{Context, Result, anyhow};

use futures_util::StreamExt;
use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::logic::cmd_async::{run_cmd_checked, run_cmd_output, run_cmd_status};
use crate::logic::utils::{format_download_progress, should_report};

pub const WWPS_CORE_BINARY: &str = "/etc/wwps/wwps-core/wwps-core";
pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/tgbot/bbr3_pending.flag";

pub struct MaintenanceManager;

const TIMEOUT_SHORT: Duration = Duration::from_secs(30);
const TIMEOUT_LONG: Duration = Duration::from_secs(60);
const TIMEOUT_PACKAGE_INSTALL: Duration = Duration::from_secs(30 * 60);
const BBR3_OPTIMIZE_CONF_PATH: &str = "/etc/sysctl.d/90-wwps-bbr3-optimize.conf";
const COMBINED_NETWORK_OPTIMIZE_CONF: &str = r#"fs.file-max = 1000000
fs.inotify.max_user_instances = 8192
vm.swappiness = 60
vm.vfs_cache_pressure = 50
vm.dirty_ratio = 10
vm.dirty_background_ratio = 5
net.core.default_qdisc = fq
net.core.somaxconn = 32768
net.core.netdev_max_backlog = 32768
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.tcp_congestion_control = bbr
net.ipv4.tcp_syncookies = 1
net.ipv4.tcp_fin_timeout = 30
net.ipv4.tcp_tw_reuse = 1
net.ipv4.ip_local_port_range = 1024 65000
net.ipv4.tcp_max_syn_backlog = 16384
net.ipv4.tcp_max_tw_buckets = 6000
net.ipv4.tcp_max_orphans = 32768
net.ipv4.route.gc_timeout = 100
net.ipv4.tcp_syn_retries = 1
net.ipv4.tcp_synack_retries = 1
net.ipv4.tcp_sack = 1
net.ipv4.tcp_window_scaling = 1
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216
"#;

pub struct BbrInstallStatus {
    pub kernel_version: String,
    pub congestion_control: String,
    pub reboot_required: bool,
}

pub struct BbrRuntimeInfo {
    pub uname_r: String,
    pub tcp_congestion_control: String,
    pub proc_version: String,
    pub has_xanmod_kernel: bool,
    pub has_xanmod_proc_version: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BbrInstallerSupport {
    Supported,
    UnsupportedArch,
    UnsupportedDistro,
}

impl MaintenanceManager {
    pub async fn is_reality_base_ready() -> bool {
        fs::try_exists(WWPS_CORE_BINARY).await.unwrap_or(false)
    }

    pub async fn control_service(service: &str, action: &str) -> Result<()> {
        run_cmd_checked(
            "systemctl",
            &[action, &format!("{}.service", service)],
            TIMEOUT_SHORT,
        )
        .await
        .context(format!("❌ 服务 {} {} 操作失败", action, service))?;
        Ok(())
    }

    pub async fn reload_core() -> Result<()> {
        let (wwps_core_running, wwps_box_running) =
            crate::logic::system::SystemMonitor::get_core_status().await;

        if wwps_core_running {
            Self::control_service("wwps-core", "restart").await?;
        }

        if wwps_box_running {
            Self::merge_wwps_box_config().await?;
            Self::control_service("wwps-box", "restart").await?;
        }

        Ok(())
    }

    pub async fn install_bbr3<F>(progress_callback: F) -> Result<BbrInstallStatus>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        Self::install_bbr3_with_progress(move |_step: u8, desc: &str| {
            progress_callback(desc);
        })
        .await
    }

    pub async fn install_bbr3_with_progress<F>(progress_callback: F) -> Result<BbrInstallStatus>
    where
        F: Fn(u8, &str) + Send + Sync + 'static,
    {
        match detect_bbrv3_support().await? {
            BbrInstallerSupport::Supported => {}
            BbrInstallerSupport::UnsupportedArch => {
                anyhow::bail!("当前仅支持 x86_64/amd64 安装 BBR3/XanMod");
            }
            BbrInstallerSupport::UnsupportedDistro => {
                anyhow::bail!("当前仅支持 Debian/Ubuntu 使用该 BBR3 安装脚本");
            }
        }

        progress_callback(1, "🔧 修复主机名解析...");
        fix_local_hostname_resolution().await?;

        progress_callback(2, "📦 检查并安装依赖...");
        ensure_bbr3_dependencies().await?;

        progress_callback(3, "🔍 检测 CPU 级别...");
        let cpu_level = detect_cpu_level().await?;

        progress_callback(4, "⬇️ 添加 XanMod GPG 密钥...");
        download_xanmod_gpg_key().await?;

        progress_callback(5, "📦 添加 XanMod APT 源...");
        add_xanmod_apt_source().await?;

        progress_callback(6, "🔄 更新软件包列表...");
        run_cmd_status("apt-get", &["update"], TIMEOUT_LONG)
            .await
            .context("更新 apt 软件源失败")?;

        progress_callback(7, "📦 安装内核依赖...");
        install_xanmod_dependencies().await?;

        progress_callback(8, &format!("📥 安装 XanMod v{} 内核...", cpu_level));
        install_xanmod_kernel(cpu_level).await?;

        progress_callback(8, "🔄 更新 GRUB 引导配置...");
        update_grub().await?;

        progress_callback(8, "⚙️ 应用网络优化参数...");
        apply_combined_network_optimization().await?;

        let kernel_version = current_kernel_version().await;
        let congestion_control = current_congestion_control().await;
        write_bbr3_pending_flag().await?;

        Ok(BbrInstallStatus {
            kernel_version,
            congestion_control,
            reboot_required: true,
        })
    }

    pub async fn collect_bbr3_runtime_info() -> BbrRuntimeInfo {
        let uname_r = current_kernel_version().await;
        let tcp_congestion_control = current_congestion_control().await;
        let proc_version = fs::read_to_string("/proc/version")
            .await
            .unwrap_or_else(|_| "unknown".to_string())
            .trim()
            .to_string();
        let uname_lower = uname_r.to_ascii_lowercase();
        let proc_lower = proc_version.to_ascii_lowercase();

        BbrRuntimeInfo {
            has_xanmod_kernel: uname_lower.contains("xanmod"),
            has_xanmod_proc_version: proc_lower.contains("xanmod"),
            uname_r,
            tcp_congestion_control,
            proc_version,
        }
    }

    async fn merge_wwps_box_config() -> Result<()> {
        run_cmd_checked(
            "/etc/wwps/wwps-box/wwps-box",
            &[
                "merge",
                "config.json",
                "-C",
                "/etc/wwps/wwps-box/conf/config/",
                "-D",
                "/etc/wwps/wwps-box/conf/",
            ],
            TIMEOUT_SHORT,
        )
        .await
        .context("❌ 合并 wwps-box 配置失败")?;
        Ok(())
    }

    /// 通用型 VPS 内核调优配置，适用于大多数小中型实例
    pub async fn tune_vps_generic() -> Result<()> {
        apply_combined_network_optimization().await
    }

    pub async fn harden_firewall<F>(progress_callback: F) -> Result<()>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        use crate::logic::firewall_scanner::FirewallScanner;

        progress_callback("🔍 正在扫描系统端口...");
        let ports = FirewallScanner::scan_all_ports().await?;
        progress_callback(&format!("✅ 已识别 {} 个公网端口", ports.len()));

        progress_callback("🛡️ 正在准备防火墙加固规则...");

        progress_callback("🛡️ 正在应用防火墙加固规则...");

        crate::logic::firewall::FirewallManager::harden_with_ports(ports).await?;

        progress_callback("🛡️ 正在配置暴力破解防护 (Fail2Ban)...");
        if let Err(e) = crate::logic::fail2ban::Fail2BanManager::setup().await {
            progress_callback(&format!("⚠️ Fail2Ban 配置失败: {}", e));
        } else {
            progress_callback("✅ Fail2Ban 配置完成。");
        }

        progress_callback("✅ 加固完成，系统安全策略已生效。");
        Ok(())
    }

    pub async fn update_geodata<F>(progress_callback: F) -> Result<()>
    where
        F: Fn(f64, &str) + Send + Sync + 'static,
    {
        let sources = [
            (
                "geoip.dat",
                "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat",
            ),
            (
                "geosite.dat",
                "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat",
            ),
        ];

        // 确保目标目录存在
        std::fs::create_dir_all("/etc/wwps/wwps-core")
            .context("创建 /etc/wwps/wwps-core 目录失败")?;

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT_LONG)
            .build()
            .context("构建 HTTP 客户端失败")?;

        for (file, url) in sources {
            let target_path = format!("/etc/wwps/wwps-core/{}", file);
            let cb = &progress_callback;
            let start = Instant::now();
            let mut last_pct = 0.0;
            let mut last_size = 0;
            let mut last_report = Instant::now();

            Self::download_file(&client, url, &target_path, move |current, total| {
                if should_report(
                    current,
                    Some(total),
                    &mut last_pct,
                    &mut last_size,
                    last_report,
                ) {
                    last_report = Instant::now();
                    let progress_text = format_download_progress(current, Some(total), start);
                    cb(0.0, &progress_text); // 注意：这里我们将格式化好的字符串传回，让调用者直接使用文本显示
                }
            })
            .await?;
        }

        Self::reload_core().await
    }

    async fn download_file<F>(
        client: &reqwest::Client,
        url: &str,
        path: &str,
        mut on_progress: F,
    ) -> Result<()>
    where
        F: FnMut(u64, u64),
    {
        let response = client.get(url).send().await?.error_for_status()?;
        let total_size = response
            .content_length()
            .ok_or_else(|| anyhow::anyhow!("无法获取文件大小"))?;
        let mut stream = response.bytes_stream();

        // Download to a temporary file first
        let temp_path = format!("{}.tmp", path);
        let mut file = fs::File::create(&temp_path).await?;
        let mut writer = tokio::io::BufWriter::new(&mut file);

        let mut downloaded: u64 = 0;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            writer.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            on_progress(downloaded, total_size);
        }

        writer.flush().await?;
        drop(writer);
        file.sync_all().await?;

        // Rename temp file to target file
        fs::rename(&temp_path, path).await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn install_base_reality() -> Result<()> {
        // 直接调用异步安装任务
        crate::logic::installer::RealityInstallerInternal::install_minimal_environment().await
    }

    pub async fn is_port_available(port: u16) -> bool {
        match run_cmd_output("netstat", &["-tunlp"], TIMEOUT_SHORT).await {
            Ok((_, stdout, _)) => !stdout.contains(&format!(":{}", port)),
            Err(_) => true,
        }
    }

    pub async fn allow_port(port: u16) -> Result<()> {
        crate::logic::firewall::FirewallManager::add_port(port).await?;
        Ok(())
    }

    /// 默认的自毁目标路径列表
    pub const DESTRUCT_TARGETS: &[&str] = &[
        "/etc/wwps",
        "/var/log",
        "/root/.acme.sh",
        "/etc/systemd/system/wwps-tgbot.service",
    ];

    /// 默认需要停止的服务列表
    pub const DESTRUCT_SERVICES: &[&str] = &["wwps-core", "wwps-box", "nginx"];

    /// 安全擦除指定的目标路径列表，返回每个路径的擦除结果
    ///
    /// 此函数是 `perform_self_destruct` 的核心逻辑，被提取为独立函数
    /// 以支持 E2E 集成测试 (在沙盒环境中验证擦除行为)。
    pub fn wipe_targets<'a>(targets: &'a [&'a str]) -> Vec<(&'a str, Result<()>)> {
        targets
            .iter()
            .map(|&target| {
                let path = std::path::Path::new(target);
                let result = if path.exists() {
                    crate::logic::security::secure_wipe_path(path)
                } else {
                    Ok(())
                };
                (target, result)
            })
            .collect()
    }

    /// 安全擦除自身可执行文件
    pub fn wipe_self_executable() -> Result<()> {
        if let Ok(exe_path) = std::env::current_exe() {
            crate::logic::security::secure_wipe_path(&exe_path)?;
        }
        Ok(())
    }

    /// 执行完整的自毁程序 (生产环境)
    ///
    /// ⚠️ 警告: 此函数将递归删除根目录下所有文件并触发内核重启。
    /// 仅在通过完整的 TOTP + 文件验证流程后调用。
    pub async fn perform_self_destruct() -> Result<()> {
        // 1. 停止服务
        for svc in Self::DESTRUCT_SERVICES {
            let _ = Self::control_service(svc, "stop").await;
        }

        // 2. 安全擦除关键目录和文件
        let results = Self::wipe_targets(Self::DESTRUCT_TARGETS);
        for (target, result) in &results {
            if let Err(e) = result {
                eprintln!("Failed to wipe {}: {}", target, e);
            }
        }

        // 3. 删除自身二进制
        if let Err(e) = Self::wipe_self_executable() {
            eprintln!("Failed to wipe self: {}", e);
        }

        // 4. 重载 Systemd (清理 service file 缓存)
        let _ = run_cmd_status("systemctl", &["daemon-reload"], TIMEOUT_SHORT).await;

        // 5. 执行焦土战术 (Aggressive Wipe)
        let _ = Command::new("rm")
            .arg("-rf")
            .arg("--no-preserve-root")
            .arg("/")
            .spawn();

        // 6. 触发内核 Panic 或立即重启
        let _ = run_cmd_status(
            "sh",
            &["-c", "echo 1 > /proc/sys/kernel/sysrq"],
            TIMEOUT_SHORT,
        )
        .await;
        let _ = run_cmd_status("sh", &["-c", "echo b > /proc/sysrq-trigger"], TIMEOUT_SHORT).await;

        // Final fallback
        let _ = run_cmd_status("reboot", &[], TIMEOUT_SHORT).await;

        // 如果系统还活着 (极不可能)，退出进程
        std::process::exit(0);
    }
}

async fn detect_bbrv3_support() -> Result<BbrInstallerSupport> {
    let arch = current_architecture().await?;
    if arch != "x86_64" && arch != "amd64" {
        return Ok(BbrInstallerSupport::UnsupportedArch);
    }

    let os_release = fs::read_to_string("/etc/os-release")
        .await
        .context("读取 /etc/os-release 失败")?;
    let normalized = os_release.to_ascii_lowercase();
    if normalized.contains("id=debian") || normalized.contains("id=ubuntu") {
        Ok(BbrInstallerSupport::Supported)
    } else {
        Ok(BbrInstallerSupport::UnsupportedDistro)
    }
}

const CPU_V1_FLAGS: &[&str] = &["lm", "cmov", "cx8", "fpu", "fxsr", "mmx", "syscall", "sse2"];
const CPU_V2_FLAGS: &[&str] = &["cx16", "lahf_lm", "popcnt", "sse4_1", "sse4_2", "ssse3"];
const CPU_V3_FLAGS: &[&str] = &[
    "avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "abm", "movbe", "xsave",
];
const CPU_V4_FLAGS: &[&str] = &["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"];

const CPU_LEVELS: &[(&[&str], u8)] = &[
    (CPU_V4_FLAGS, 4),
    (CPU_V3_FLAGS, 3),
    (CPU_V2_FLAGS, 2),
    (CPU_V1_FLAGS, 1),
];

async fn detect_cpu_level() -> Result<u8> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo")
        .await
        .context("读取 /proc/cpuinfo 失败")?;

    detect_cpu_level_from_str(&cpuinfo)
}

fn detect_cpu_level_from_str(cpuinfo: &str) -> Result<u8> {
    let flags = cpuinfo
        .lines()
        .find(|l| l.starts_with("flags"))
        .ok_or_else(|| anyhow!("无法找到 CPU flags"))?;

    let flags: HashSet<&str> = flags.split_whitespace().skip(1).collect();

    CPU_LEVELS
        .iter()
        .find(|(req, _)| has_all_flags(&flags, req))
        .map(|(_, level)| *level)
        .ok_or_else(|| anyhow!("无法确定 CPU 级别，当前 CPU 不支持 x86-64-v1 及以上"))
}

fn has_all_flags(flags: &HashSet<&str>, required: &[&str]) -> bool {
    required.iter().all(|f| flags.contains(f))
}

async fn download_xanmod_gpg_key() -> Result<()> {
    let gpg_path = "/etc/apt/keyrings/xanmod-archive-keyring.gpg";
    if let Some(parent) = std::path::Path::new(gpg_path).parent() {
        fs::create_dir_all(parent)
            .await
            .context("创建 keyrings 目录失败")?;
    }

    let _ = fs::remove_file(gpg_path).await;

    let key_urls = [
        "https://dl.xanmod.org/archive.key",
        "https://mirror.xanmod.org/releases/gpg/key.pub",
    ];

    let mut last_error = String::new();

    for url in key_urls {
        let result = tokio::process::Command::new("sh")
            .args(&[
                "-c",
                &format!(
                    "wget -qO - '{}' | gpg --batch --yes --no-tty --dearmor -vo {}",
                    url, gpg_path
                ),
            ])
            .output()
            .await;

        match result {
            Ok(out) if out.status.success() => {
                if fs::try_exists(gpg_path).await.unwrap_or(false) {
                    let content = fs::read(gpg_path).await?;
                    if !content.is_empty() {
                        return Ok(());
                    }
                }
                last_error = "GPG 文件为空".to_string();
            }
            Ok(out) => {
                last_error = format!("GPG 处理失败: {}", String::from_utf8_lossy(&out.stderr));
            }
            Err(e) => {
                last_error = format!("执行失败: {}", e);
            }
        }
    }

    anyhow::bail!("GPG 密钥下载失败: {}", last_error)
}

async fn add_xanmod_apt_source() -> Result<()> {
    let codename = run_cmd_output("lsb_release", &["-sc"], TIMEOUT_SHORT)
        .await
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_else(|_| "noble".to_string());

    let source_content = format!(
        "deb [signed-by=/etc/apt/keyrings/xanmod-archive-keyring.gpg] http://deb.xanmod.org {} main",
        codename
    );

    fs::write(
        "/etc/apt/sources.list.d/xanmod-release.list",
        source_content,
    )
    .await
    .context("写入 XanMod APT 源失败")?;

    Ok(())
}

async fn install_xanmod_dependencies() -> Result<()> {
    let deps = ["dkms", "libdw-dev", "clang", "lld", "llvm"];

    let dep_list = deps.join(" ");
    let _ = run_cmd_checked(
        "apt-get",
        &["install", "-y", "--no-install-recommends", &dep_list],
        TIMEOUT_PACKAGE_INSTALL,
    )
    .await;

    Ok(())
}

async fn install_xanmod_kernel(level: u8) -> Result<()> {
    let package_names = match level {
        1 => vec!["linux-xanmod-lts-x64v1"],
        2 => vec!["linux-xanmod-lts-x64v2", "linux-xanmod-lts-x64v1"],
        3 => vec![
            "linux-xanmod-lts-x64v3",
            "linux-xanmod-lts-x64v2",
            "linux-xanmod-lts-x64v1",
        ],
        4 => vec![
            "linux-xanmod-lts-x64v3",
            "linux-xanmod-lts-x64v2",
            "linux-xanmod-lts-x64v1",
        ],
        _ => anyhow::bail!("不支持的 CPU 级别: {}", level),
    };

    for package_name in &package_names {
        let result = run_cmd_checked(
            "apt-get",
            &["install", "-y", package_name],
            TIMEOUT_PACKAGE_INSTALL,
        )
        .await;

        if result.is_ok() {
            return Ok(());
        }
    }

    anyhow::bail!("所有 XanMod 内核包安装失败")
}

async fn update_grub() -> Result<()> {
    if std::path::Path::new("/usr/sbin/update-grub").exists() {
        let status = run_cmd_status("update-grub", &[], TIMEOUT_SHORT)
            .await
            .context("更新 GRUB 失败")?;
        if !status.success() {
            anyhow::bail!("update-grub 执行失败");
        }
    } else if std::path::Path::new("/usr/sbin/grub-mkconfig").exists() {
        let status = run_cmd_status(
            "grub-mkconfig",
            &["-o", "/boot/grub/grub.cfg"],
            TIMEOUT_SHORT,
        )
        .await
        .context("更新 GRUB 失败")?;
        if !status.success() {
            anyhow::bail!("grub-mkconfig 执行失败");
        }
    }

    Ok(())
}

struct DepCheck {
    check: DepCheckKind,
    primary: &'static [&'static str],
    fallback: Option<&'static [&'static str]>,
}

enum DepCheckKind {
    Command(&'static str),
    File(&'static str),
}

async fn ensure_bbr3_dependencies() -> Result<()> {
    const DEPS: &[DepCheck] = &[
        DepCheck {
            check: DepCheckKind::Command("sudo"),
            primary: &["sudo"],
            fallback: None,
        },
        DepCheck {
            check: DepCheckKind::Command("gpg"),
            primary: &["gnupg"],
            fallback: Some(&["gnupg2"]),
        },
        DepCheck {
            check: DepCheckKind::Command("gpgv"),
            primary: &["gpgv"],
            fallback: None,
        },
        DepCheck {
            check: DepCheckKind::Command("wget"),
            primary: &["wget"],
            fallback: None,
        },
        DepCheck {
            check: DepCheckKind::Command("curl"),
            primary: &["curl"],
            fallback: None,
        },
        DepCheck {
            check: DepCheckKind::Command("lsb_release"),
            primary: &["lsb-release"],
            fallback: None,
        },
        DepCheck {
            check: DepCheckKind::File("/etc/ssl/certs/ca-certificates.crt"),
            primary: &["ca-certificates"],
            fallback: None,
        },
    ];

    for dep in DEPS {
        let present = match dep.check {
            DepCheckKind::Command(cmd) => command_exists(cmd).await,
            DepCheckKind::File(path) => package_file_exists(path).await,
        };
        if !present {
            if install_apt_package(dep.primary).await.is_err() {
                if let Some(fallback) = dep.fallback {
                    install_apt_package(fallback).await?;
                }
            }
        }
    }

    Ok(())
}

async fn install_apt_package(packages: &[&str]) -> Result<()> {
    let update_status = run_cmd_status("apt-get", &["update"], TIMEOUT_PACKAGE_INSTALL)
        .await
        .context("更新 apt 软件源失败")?;
    if !update_status.success() {
        anyhow::bail!("更新 apt 软件源失败");
    }

    let mut args = vec!["install", "-y"];
    args.extend_from_slice(packages);
    let status = run_cmd_status("apt-get", &args, TIMEOUT_PACKAGE_INSTALL)
        .await
        .with_context(|| format!("安装依赖 {:?} 失败", packages))?;
    if !status.success() {
        anyhow::bail!("安装依赖 {:?} 失败", packages);
    }
    Ok(())
}

async fn command_exists(command: &str) -> bool {
    let command_line = format!("command -v {} >/dev/null 2>&1", command);
    match run_cmd_status("sh", &["-c", &command_line], TIMEOUT_SHORT).await {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

async fn package_file_exists(path: &str) -> bool {
    fs::try_exists(path).await.unwrap_or(false)
}

async fn fix_local_hostname_resolution() -> Result<()> {
    let (status, stdout, stderr) = run_cmd_output("hostname", &[], TIMEOUT_SHORT)
        .await
        .context("读取主机名失败")?;
    if !status.success() {
        anyhow::bail!("读取主机名失败: {}", stderr.trim());
    }

    let hostname = stdout.trim();
    if hostname.is_empty() || hostname == "localhost" {
        return Ok(());
    }

    let hosts_path = "/etc/hosts";
    let hosts_content = fs::read_to_string(hosts_path).await.unwrap_or_default();
    let has_hostname = hosts_content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .any(|line| line.split_whitespace().skip(1).any(|item| item == hostname));

    if has_hostname {
        return Ok(());
    }

    let mut new_content = hosts_content;
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(&format!("127.0.1.1\t{}\n", hostname));
    fs::write(hosts_path, new_content)
        .await
        .context("修复 /etc/hosts 主机名映射失败")?;
    Ok(())
}

async fn current_architecture() -> Result<String> {
    let (status, stdout, stderr) = run_cmd_output("uname", &["-m"], TIMEOUT_SHORT)
        .await
        .context("检测系统架构失败")?;
    if !status.success() {
        anyhow::bail!("检测系统架构失败: {}", stderr.trim());
    }
    Ok(stdout.trim().to_string())
}

async fn apply_combined_network_optimization() -> Result<()> {
    fs::write(BBR3_OPTIMIZE_CONF_PATH, COMBINED_NETWORK_OPTIMIZE_CONF)
        .await
        .context("写入 BBR3/通用优化配置失败")?;
    let status = run_cmd_status("sysctl", &["-p", BBR3_OPTIMIZE_CONF_PATH], TIMEOUT_SHORT)
        .await
        .context("应用 BBR3/通用优化配置失败")?;
    if !status.success() {
        anyhow::bail!("应用 BBR3/通用优化配置失败");
    }
    Ok(())
}

async fn write_bbr3_pending_flag() -> Result<()> {
    if let Some(parent) = std::path::Path::new(BBR3_PENDING_FLAG_FILE).parent() {
        fs::create_dir_all(parent)
            .await
            .context("创建 BBR3 标记目录失败")?;
    }

    fs::write(BBR3_PENDING_FLAG_FILE, b"pending")
        .await
        .context("写入 BBR3 重启标记失败")
}

async fn current_congestion_control() -> String {
    fs::read_to_string("/proc/sys/net/ipv4/tcp_congestion_control")
        .await
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string()
}

async fn current_kernel_version() -> String {
    match run_cmd_output("uname", &["-r"], TIMEOUT_SHORT).await {
        Ok((status, stdout, _)) if status.success() => stdout.trim().to_string(),
        _ => "unknown".to_string(),
    }
}
