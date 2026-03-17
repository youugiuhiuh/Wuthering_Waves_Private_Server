use anyhow::{Context, Result};

use futures_util::StreamExt;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::logic::cmd_async::{run_cmd_checked, run_cmd_output, run_cmd_status};
use crate::logic::utils::{format_download_progress, should_report};

pub const WWPS_CORE_BINARY: &str = "/etc/wwps/wwps-core/wwps-core";
pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/tgbot/bbr3_pending.flag";
const PRIMARY_BBR_SCRIPT_URL: &str =
    "https://raw.githubusercontent.com/opiran-club/VPS-Optimizer/main/bbrv3.sh";
const PRIMARY_BBR_SCRIPT_PATH: &str = "/tmp/wwps-bbrv3.sh";
const BACKUP_BBR_SCRIPT_URL: &str =
    "https://raw.githubusercontent.com/jinwyp/one_click_script/master/install_kernel.sh";
const BACKUP_BBR_SCRIPT_PATH: &str = "/tmp/wwps-install_kernel.sh";

pub struct MaintenanceManager;

const TIMEOUT_SHORT: Duration = Duration::from_secs(30);
const TIMEOUT_LONG: Duration = Duration::from_secs(60);
const TIMEOUT_BBR_INSTALL: Duration = Duration::from_secs(30 * 60);
const TIMEOUT_PACKAGE_INSTALL: Duration = Duration::from_secs(10 * 60);
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

#[derive(Clone, Copy)]
struct BbrScriptSource {
    name: &'static str,
    url: &'static str,
    path: &'static str,
    input_builder: fn() -> String,
}

const PRIMARY_BBR_SOURCE: BbrScriptSource = BbrScriptSource {
    name: "opiran-bbrv3",
    url: PRIMARY_BBR_SCRIPT_URL,
    path: PRIMARY_BBR_SCRIPT_PATH,
    input_builder: build_bbrv3_script_input,
};

const BACKUP_BBR_SOURCE: BbrScriptSource = BbrScriptSource {
    name: "jinwyp-install-kernel",
    url: BACKUP_BBR_SCRIPT_URL,
    path: BACKUP_BBR_SCRIPT_PATH,
    input_builder: build_backup_bbr_script_input,
};

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
        match detect_bbrv3_support().await? {
            BbrInstallerSupport::Supported => {}
            BbrInstallerSupport::UnsupportedArch => {
                anyhow::bail!("当前仅支持 x86_64/amd64 安装 BBR3/XanMod");
            }
            BbrInstallerSupport::UnsupportedDistro => {
                anyhow::bail!("当前仅支持 Debian/Ubuntu 使用该 BBR3 安装脚本");
            }
        }

        progress_callback("🔧 修复主机名解析...");
        fix_local_hostname_resolution().await?;

        progress_callback("📦 检查并安装依赖...");
        ensure_bbr3_dependencies().await?;

        progress_callback("⬇️ 下载并安装 BBR3/XanMod 内核...");
        run_bbr_install_flow().await?;

        progress_callback("⚙️ 应用网络优化参数...");
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
        let mut results = Vec::new();
        for target in targets {
            let path = std::path::Path::new(target);
            if path.exists() {
                let result = crate::logic::security::secure_wipe_path(path);
                results.push((*target, result));
            } else {
                results.push((*target, Ok(())));
            }
        }
        results
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

async fn ensure_bbr3_dependencies() -> Result<()> {
    if !command_exists("sudo").await {
        install_apt_package(&["sudo"]).await?;
    }

    if !command_exists("gpg").await {
        if install_apt_package(&["gnupg"]).await.is_err() {
            install_apt_package(&["gnupg2"]).await?;
        }
    }

    if !command_exists("gpgv").await {
        install_apt_package(&["gpgv"]).await?;
    }

    if !command_exists("wget").await {
        install_apt_package(&["wget"]).await?;
    }

    if !command_exists("curl").await {
        install_apt_package(&["curl"]).await?;
    }

    if !command_exists("lsb_release").await {
        install_apt_package(&["lsb-release"]).await?;
    }

    if !package_file_exists("/etc/ssl/certs/ca-certificates.crt").await {
        install_apt_package(&["ca-certificates"]).await?;
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

async fn run_bbr_install_flow() -> Result<()> {
    match run_script_source(PRIMARY_BBR_SOURCE).await {
        Ok(()) => Ok(()),
        Err(primary_err) => match run_script_source(BACKUP_BBR_SOURCE).await {
            Ok(()) => Ok(()),
            Err(backup_err) => {
                anyhow::bail!("主脚本失败: {}; 备用脚本失败: {}", primary_err, backup_err)
            }
        },
    }
}

async fn run_script_source(source: BbrScriptSource) -> Result<()> {
    download_script(source).await?;
    let status = run_script(source).await?;
    if !status.success() {
        anyhow::bail!("{} 执行失败", source.name);
    }
    if !has_xanmod_installation_markers().await? {
        anyhow::bail!("{} 执行完成，但未检测到 XanMod 安装痕迹", source.name);
    }
    Ok(())
}

async fn download_script(source: BbrScriptSource) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT_LONG)
        .build()
        .context("构建 BBR3 下载客户端失败")?;
    let content = client
        .get(source.url)
        .send()
        .await
        .context("下载 BBR3 脚本失败")?
        .error_for_status()
        .context("BBR3 脚本源返回错误状态")?
        .text()
        .await
        .context("读取 BBR3 脚本内容失败")?;

    fs::write(source.path, content)
        .await
        .context("写入 BBR3 脚本失败")?;

    let chmod_status = run_cmd_status("chmod", &["+x", source.path], TIMEOUT_SHORT)
        .await
        .context("设置 BBR3 脚本权限失败")?;
    if !chmod_status.success() {
        anyhow::bail!("设置 BBR3 脚本权限失败");
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

async fn run_script(source: BbrScriptSource) -> Result<std::process::ExitStatus> {
    let mut child = tokio::process::Command::new("bash")
        .arg(source.path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("启动 BBR3 安装脚本失败")?;

    let mut stdin = child.stdin.take().context("无法打开 BBR3 脚本 stdin")?;
    let script_input = (source.input_builder)();
    tokio::spawn(async move {
        let _ = stdin.write_all(script_input.as_bytes()).await;
    });

    let output = tokio::time::timeout(TIMEOUT_BBR_INSTALL, child.wait_with_output())
        .await
        .context("执行 BBR3 安装脚本超时")?
        .context("等待 BBR3 安装脚本结束失败")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        anyhow::bail!("BBR3 脚本失败: {}", combined);
    }

    Ok(output.status)
}

fn build_bbrv3_script_input() -> String {
    [
        "1", // 安装 XanMod + BBRv3
        "y", // 确认安装
        "",  // continue after kernel installation
        "y", // update-grub
        "y", // apply sysctl tuning
        "n", // do not reboot immediately
        "",  // continue loop
        "e", // exit script menu
    ]
    .join("\n")
        + "\n"
}

fn build_backup_bbr_script_input() -> String {
    [
        "52", // 安装 XanMod 6.11
        "y",  // continue operation if prompted
        "n",  // do not reboot immediately
        "0",  // exit menu
    ]
    .join("\n")
        + "\n"
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

async fn has_xanmod_installation_markers() -> Result<bool> {
    if dir_contains_xanmod("/boot").await? {
        return Ok(true);
    }

    if dir_contains_xanmod("/lib/modules").await? {
        return Ok(true);
    }

    if file_contains_xanmod("/boot/grub/grub.cfg").await? {
        return Ok(true);
    }

    if file_contains_xanmod("/boot/grub2/grub.cfg").await? {
        return Ok(true);
    }

    Ok(false)
}

async fn dir_contains_xanmod(path: &str) -> Result<bool> {
    let exists = fs::try_exists(path).await.unwrap_or(false);
    if !exists {
        return Ok(false);
    }

    let mut entries = fs::read_dir(path)
        .await
        .with_context(|| format!("读取目录 {} 失败", path))?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        if name
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("xanmod")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn file_contains_xanmod(path: &str) -> Result<bool> {
    let exists = fs::try_exists(path).await.unwrap_or(false);
    if !exists {
        return Ok(false);
    }

    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("读取文件 {} 失败", path))?;
    Ok(content.to_ascii_lowercase().contains("xanmod"))
}

#[cfg(test)]
mod tests {
    use super::{build_backup_bbr_script_input, build_bbrv3_script_input};

    #[test]
    fn test_bbrv3_script_input_sequence() {
        let input = build_bbrv3_script_input();
        assert!(input.starts_with("1\ny\n"));
        assert!(input.contains("\ny\ny\nn\n"));
        assert!(input.ends_with("\ne\n"));
    }

    #[test]
    fn test_backup_bbr_script_input_sequence() {
        let input = build_backup_bbr_script_input();
        assert!(input.starts_with("52\ny\n"));
        assert!(input.ends_with("\n0\n"));
    }
}
