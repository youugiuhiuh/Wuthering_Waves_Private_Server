use anyhow::{Context, Result, anyhow};

use futures_util::StreamExt;
use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::core::cmd_action::ServiceAction;
use crate::core::cmd_async::{run_cmd_checked, run_cmd_output, run_cmd_status};
use crate::core::paths::maintenance::{
    BBR3_PENDING_FLAG_FILE, DESTRUCT_SERVICES, DESTRUCT_TARGETS,
};
use crate::core::paths::xray;
use crate::core::utils::{format_download_progress, should_report};
use crate::core::xray::installer::PackageManager;

fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let start = s.len() - max_len;
        format!("…\n{}", &s[start..])
    }
}

async fn run_apt_stage<F>(
    progress_callback: &F,
    desc: &'static str,
    fail_context: &'static str,
    args: &[&str],
) -> Result<()>
where
    F: Fn(&str) + Send + Sync,
{
    progress_callback(desc);
    let (stdout, stderr) = run_cmd_checked("apt-get", args, TIMEOUT_PACKAGE_INSTALL)
        .await
        .context(fail_context)?;
    let output = format!("{}{}", stdout.trim(), stderr.trim());
    if !output.is_empty() {
        progress_callback(&truncate_output(&output, OUTPUT_MAX));
    }
    Ok(())
}

pub struct MaintenanceManager;

const TIMEOUT_SHORT: Duration = Duration::from_secs(30);
const TIMEOUT_LONG: Duration = Duration::from_secs(60);
const TIMEOUT_PACKAGE_INSTALL: Duration = Duration::from_secs(30 * 60);
const OUTPUT_MAX: usize = 3000;
const BBR3_OPTIMIZE_CONF_PATH: &str = "/etc/sysctl.d/90-wwps-bbr3-optimize.conf";
const NETWORK_OPTIMIZE_CONF: &str = r#"fs.file-max = 1000000
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
net.ipv4.tcp_max_tw_buckets = 32768
net.ipv4.tcp_max_orphans = 32768
net.ipv4.route.gc_timeout = 100
net.ipv4.tcp_syn_retries = 1
net.ipv4.tcp_synack_retries = 1
net.ipv4.tcp_sack = 1
net.ipv4.tcp_window_scaling = 1
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216
net.netfilter.nf_conntrack_max = 262144
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
        fs::try_exists(xray::BIN).await.unwrap_or(false)
    }

    pub async fn control_service(service: &str, action: ServiceAction) -> Result<()> {
        run_cmd_checked(
            "systemctl",
            &[action.as_str(), &format!("{}.service", service)],
            TIMEOUT_SHORT,
        )
        .await
        .context(format!("❌ 服务 {} {} 操作失败", action, service))?;
        Ok(())
    }

    pub async fn reload_core() -> Result<()> {
        let (wwps_core_running, wwps_box_running) =
            crate::core::system::SystemMonitor::get_core_status().await;

        if wwps_core_running {
            crate::core::xray::config::ConfigManager::ensure_base_config().await?;
            Self::control_service("wwps-core", ServiceAction::Restart).await?;
        }

        if wwps_box_running {
            crate::core::singbox::SingBoxConfigManager::ensure_base_config().await?;
            Self::control_service("wwps-box", ServiceAction::Restart).await?;
        }

        // Sync firewall rules after restart: remove stale ports
        if let Err(e) = Self::sync_firewall_with_configs().await {
            log::error!("防火墙端口同步失败: {}", e);
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

        progress_callback(9, "⚙️ 写入网络优化配置...");
        let _ = apply_network_optimization().await;

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

    /// 通用型 VPS 内核调优配置，适用于大多数小中型实例
    pub async fn tune_vps_generic() -> Result<()> {
        apply_network_optimization().await
    }

    pub async fn upgrade_system_packages<F>(progress_callback: F) -> Result<()>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let pm = PackageManager::detect().await?;
        if !matches!(pm, PackageManager::Apt) {
            anyhow::bail!("系统更新功能目前仅支持 Debian/Ubuntu");
        }

        let cb = &progress_callback;

        run_apt_stage(
            cb,
            "🔄 正在刷新软件包索引...",
            "更新软件包索引失败",
            &["update"],
        )
        .await?;
        run_apt_stage(
            cb,
            "⬆️ 正在执行系统全面升级...",
            "系统升级失败",
            &["full-upgrade", "-y"],
        )
        .await?;
        run_apt_stage(
            cb,
            "🧹 正在清理无用依赖...",
            "自动移除无用依赖失败",
            &["autoremove", "-y"],
        )
        .await?;
        run_apt_stage(cb, "🧹 正在清理包缓存...", "清理包缓存失败", &["autoclean"]).await?;

        progress_callback("✅ 系统软件包更新完成");
        Ok(())
    }

    pub async fn harden_firewall<F>(progress_callback: F) -> Result<()>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        use crate::core::security::firewall_scanner::FirewallScanner;

        progress_callback("🔍 正在扫描系统端口...");
        let ports = FirewallScanner::scan_all_ports().await?;
        progress_callback(&format!("✅ 已识别 {} 个公网端口", ports.len()));

        progress_callback("🛡️ 正在准备防火墙加固规则...");

        progress_callback("🛡️ 正在应用防火墙加固规则...");

        crate::core::security::firewall::FirewallManager::harden_with_ports(ports).await?;

        progress_callback("🛡️ 正在配置暴力破解防护 (Fail2Ban)...");
        if let Err(e) = crate::core::security::fail2ban::Fail2BanManager::setup().await {
            progress_callback(&format!("⚠️ Fail2Ban 配置失败: {}", e));
        } else {
            progress_callback("✅ Fail2Ban 配置完成。");
        }

        progress_callback("✅ 加固完成，系统安全策略已生效。");
        Ok(())
    }

    pub async fn ensure_geodata() -> Result<()> {
        Self::ensure_geodata_in_dir(xray::DIR).await
    }

    pub async fn ensure_geodata_in_dir(dir: &str) -> Result<()> {
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

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT_LONG)
            .build()
            .context("构建 HTTP 客户端失败")?;

        for (file, url) in sources {
            let target_path = format!("{}/{}", dir, file);

            if tokio::fs::try_exists(&target_path).await.unwrap_or(false) {
                log::info!("geodata 文件已存在，跳过: {}", target_path);
                continue;
            }

            log::info!("正在下载 geodata: {}", file);

            if let Err(e) = Self::download_file_atomic(&client, url, &target_path).await {
                log::warn!("下载 {} 失败: {} (服务可能仍可启动)", file, e);
            }
        }

        Ok(())
    }

    async fn download_file_atomic(client: &reqwest::Client, url: &str, path: &str) -> Result<()> {
        let temp_path = format!("{}.tmp", path);

        let result = Self::download_file(client, url, &temp_path, |_, _| {}).await;

        match result {
            Ok(()) => {
                tokio::fs::rename(&temp_path, path)
                    .await
                    .context("重命名临时文件失败")?;
                log::info!("geodata 下载完成: {}", path);
                Ok(())
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Err(e)
            }
        }
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
        std::fs::create_dir_all(xray::DIR).context("创建 xray 目录失败")?;

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT_LONG)
            .build()
            .context("构建 HTTP 客户端失败")?;

        for (file, url) in sources {
            let target_path = format!("{}/{}", xray::DIR, file);
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

        // sing-box 域名规则集（每日更新源，跟随本任务）
        if let Err(e) = Self::update_singbox_rules(false, move |_pct, msg| {
            progress_callback(0.0, &format!("[Sing-box] {}", msg));
        })
        .await
        {
            log::warn!("更新 sing-box 规则集失败: {}", e);
        }

        Self::reload_core().await
    }

    /// sing-box 规则集：下载官方 .db → geosite/geoip export → rule-set compile 生成 .srs
    pub async fn update_singbox_rules<F>(include_geoip: bool, progress_callback: F) -> Result<()>
    where
        F: Fn(f64, &str) + Send + Sync + 'static,
    {
        use crate::core::paths::singbox;

        // 无 sing-box 的主机不必下载/转换（如仅部署 Xray 的机器）
        if !std::path::Path::new(singbox::BIN).exists() {
            return Ok(());
        }

        let rule_dir = singbox::RULE_SET_DIR;
        std::fs::create_dir_all(rule_dir).context("创建 rule-set 目录失败")?;
        let temp_dir = format!("{}/.update-tmp", singbox::DIR);
        std::fs::create_dir_all(&temp_dir).context("创建临时目录失败")?;

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT_LONG)
            .build()
            .context("构建 HTTP 客户端失败")?;

        // 1. 下载 geosite.db（域名库，总是更新）
        let geosite_db = format!("{}/geosite.db", temp_dir);
        progress_callback(0.0, "下载 geosite.db (官方每日更新)...");
        Self::download_file(
            &client,
            "https://github.com/SagerNet/sing-geosite/releases/latest/download/geosite.db",
            &geosite_db,
            |_, _| {},
        )
        .await
        .context("下载 geosite.db 失败")?;

        // 2. 可选下载 geoip.db
        let geoip_db = format!("{}/geoip.db", temp_dir);
        if include_geoip {
            progress_callback(0.0, "下载 geoip.db (官方每月更新)...");
            Self::download_file(
                &client,
                "https://github.com/SagerNet/sing-geoip/releases/latest/download/geoip.db",
                &geoip_db,
                |_, _| {},
            )
            .await
            .context("下载 geoip.db 失败")?;
        }

        // 3. 转换 geosite 分类
        for cat in crate::core::singbox::routing::GEOSITE_CATEGORIES {
            let out_json = format!("{}/geosite-{}.json", temp_dir, cat);
            let out_srs = format!("{}/geosite-{}.srs", rule_dir, cat);
            progress_callback(0.0, &format!("转换 geosite-{}...", cat));
            Self::run_singbox_convert(&geosite_db, &out_json, &out_srs, "geosite", cat).await?;
        }

        // 4. 转换 geoip 分类
        if include_geoip {
            for cat in crate::core::singbox::routing::GEOIP_CATEGORIES {
                let out_json = format!("{}/geoip-{}.json", temp_dir, cat);
                let out_srs = format!("{}/geoip-{}.srs", rule_dir, cat);
                progress_callback(0.0, &format!("转换 geoip-{}...", cat));
                Self::run_singbox_convert(&geoip_db, &out_json, &out_srs, "geoip", cat).await?;
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
        progress_callback(1.0, "sing-box 规则集更新完成");
        Self::reload_core().await
    }

    /// 安装时确保规则集存在（skip-if-exists）
    pub async fn ensure_singbox_rule_sets() -> Result<()> {
        use crate::core::paths::singbox;
        let rule_dir = singbox::RULE_SET_DIR;
        std::fs::create_dir_all(rule_dir).context("创建 rule-set 目录失败")?;
        let missing = Self::singbox_missing_srs(rule_dir);
        if missing.is_empty() {
            return Ok(());
        }
        Self::update_singbox_rules(true, |_, _| {}).await
    }

    /// 返回缺失的 (kind, category) 列表
    pub fn singbox_missing_srs(rule_dir: &str) -> Vec<(&'static str, &'static str)> {
        use crate::core::singbox::routing::{GEOIP_CATEGORIES, GEOSITE_CATEGORIES};
        let mut missing = Vec::new();
        for &cat in GEOSITE_CATEGORIES {
            if !std::path::Path::new(&format!("{}/geosite-{}.srs", rule_dir, cat)).exists() {
                missing.push(("geosite", cat));
            }
        }
        for &cat in GEOIP_CATEGORIES {
            if !std::path::Path::new(&format!("{}/geoip-{}.srs", rule_dir, cat)).exists() {
                missing.push(("geoip", cat));
            }
        }
        missing
    }

    /// 生成 geosite/geoip export + rule-set compile 两条命令
    /// `kind` 为 "geosite" 或 "geoip"（决定子命令，geoip.db 必须用 `geoip export`）
    pub fn singbox_convert_args(
        program: &str,
        kind: &str,
        category: &str,
        db_path: &str,
        out_json: &str,
        out_srs: &str,
    ) -> Vec<Vec<String>> {
        vec![
            vec![
                program.to_string(),
                kind.to_string(),
                "export".to_string(),
                category.to_string(),
                "-f".to_string(),
                db_path.to_string(),
                "-o".to_string(),
                out_json.to_string(),
            ],
            vec![
                program.to_string(),
                "rule-set".to_string(),
                "compile".to_string(),
                "--output".to_string(),
                out_srs.to_string(),
                out_json.to_string(),
            ],
        ]
    }

    async fn run_singbox_convert(
        db_path: &str,
        out_json: &str,
        out_srs: &str,
        kind: &str,
        category: &str,
    ) -> Result<()> {
        use crate::core::paths::singbox;
        let cmds =
            Self::singbox_convert_args(singbox::BIN, kind, category, db_path, out_json, out_srs);
        for args in &cmds {
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_cmd_checked(arg_refs[0], &arg_refs[1..], TIMEOUT_LONG)
                .await
                .with_context(|| format!("sing-box 转换命令失败: {:?}", args))?;
        }
        Ok(())
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

    pub async fn is_port_available(port: u16) -> bool {
        match run_cmd_output("netstat", &["-tunlp"], TIMEOUT_SHORT).await {
            Ok((_, stdout, _)) => !stdout.contains(&format!(":{}", port)),
            Err(_) => true,
        }
    }

    pub async fn allow_port(port: u16) -> Result<()> {
        crate::core::security::firewall::FirewallManager::add_port(port).await?;
        Ok(())
    }

    pub async fn remove_port(port: u16) -> Result<()> {
        crate::core::security::firewall::FirewallManager::remove_port(port).await?;
        Ok(())
    }

    pub async fn sync_firewall_with_configs() -> Result<()> {
        use std::collections::HashSet;

        let xray_ports = crate::core::xray::ConfigManager::collect_all_ports()
            .await
            .unwrap_or_default();

        let singbox_ports = crate::core::singbox::SingBoxConfigManager::collect_all_ports()
            .await
            .unwrap_or_default();

        let mut required: HashSet<u16> = HashSet::new();
        required.extend(xray_ports);
        required.extend(singbox_ports);
        required.insert(22); // SSH

        let current = crate::core::security::firewall::FirewallManager::list_allowed_ports()
            .await
            .unwrap_or_default();

        let stale: HashSet<u16> = current.difference(&required).copied().collect();

        if stale.is_empty() {
            log::info!("防火墙端口同步: 无需清理");
            return Ok(());
        }

        log::warn!(
            "防火墙端口同步: 发现 {} 个过期端口待清理: {:?}",
            stale.len(),
            stale.iter().take(20).collect::<Vec<_>>()
        );

        let mut removed = 0u32;
        for port in &stale {
            match crate::core::security::firewall::FirewallManager::remove_port(*port).await {
                Ok(()) => {
                    removed += 1;
                    log::info!("防火墙: 已移除过期端口 {}", port);
                }
                Err(e) => {
                    log::warn!("防火墙: 移除端口 {} 失败: {}", port, e);
                }
            }
        }

        log::info!(
            "防火墙端口同步完成: 移除 {} / {} 个过期端口",
            removed,
            stale.len()
        );

        Ok(())
    }

    pub async fn is_ufw_active() -> bool {
        crate::core::security::ufw::UfwClient::is_active().await
    }

    pub async fn is_firewalld_active() -> bool {
        crate::core::security::firewalld::FirewalldClient::is_active().await
    }

    /// 允许端口范围（IPv4）
    /// 检测 ufw 和 firewalld 是否激活，然后调用相应的方法添加端口范围
    pub async fn allow_port_range(start: u16, end: u16) -> Result<()> {
        // 检测 ufw 是否激活
        if Self::is_ufw_active().await {
            crate::core::security::ufw::UfwClient::add_port_range(start, end, "udp").await?;
        }

        // 检测 firewalld 是否激活
        if Self::is_firewalld_active().await {
            crate::core::security::firewalld::FirewalldClient::add_port_range(start, end, "udp")
                .await?;
        }

        Ok(())
    }

    /// 允许端口范围（IPv6）
    /// firewalld 自动处理 IPv6，无需额外调用
    pub async fn allow_port_range_v6(start: u16, end: u16) -> Result<()> {
        // 检测 ufw 是否激活
        if Self::is_ufw_active().await {
            crate::core::security::ufw::UfwClient::add_port_range_v6(start, end, "udp").await?;
        }

        // firewalld 自动处理 IPv6，无需额外调用

        Ok(())
    }

    /// 移除端口范围（IPv4）
    /// 检测 ufw 和 firewalld 是否激活，然后调用相应的方法移除端口范围
    pub async fn remove_port_range(start: u16, end: u16) -> Result<()> {
        // 检测 ufw 是否激活
        if Self::is_ufw_active().await {
            crate::core::security::ufw::UfwClient::remove_port_range(start, end, "udp").await?;
        }

        // 检测 firewalld 是否激活
        if Self::is_firewalld_active().await {
            crate::core::security::firewalld::FirewalldClient::remove_port_range(start, end, "udp")
                .await?;
        }

        Ok(())
    }

    /// 移除端口范围（IPv6）
    pub async fn remove_port_range_v6(start: u16, end: u16) -> Result<()> {
        // 检测 ufw 是否激活
        if Self::is_ufw_active().await {
            crate::core::security::ufw::UfwClient::remove_port_range_v6(start, end, "udp").await?;
        }

        Ok(())
    }

    /// 默认的自毁目标路径列表
    pub const DESTRUCT_TARGETS: &[&str] = DESTRUCT_TARGETS;
    pub const DESTRUCT_SERVICES: &[&str] = DESTRUCT_SERVICES;

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
                    crate::core::security::secure_wipe_path(path)
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
            crate::core::security::secure_wipe_path(&exe_path)?;
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
            let _ = Self::control_service(svc, ServiceAction::Stop).await;
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
            .args([
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
        if !present
            && install_apt_package(dep.primary).await.is_err()
            && let Some(fallback) = dep.fallback
        {
            install_apt_package(fallback).await?;
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

async fn apply_network_optimization() -> Result<()> {
    fs::write(BBR3_OPTIMIZE_CONF_PATH, NETWORK_OPTIMIZE_CONF)
        .await
        .context("写入网络优化配置失败")?;
    let status = run_cmd_status(
        "sysctl",
        &["-e", "-p", BBR3_OPTIMIZE_CONF_PATH],
        TIMEOUT_SHORT,
    )
    .await
    .context("执行 sysctl 失败")?;
    if !status.success() {
        log::warn!(
            "部分 sysctl 参数未生效（可能不兼容当前内核），重启后将由 systemd-sysctl 自动加载"
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_has_all_flags_all_present() {
        let flags: HashSet<&str> = vec!["lm", "sse2", "avx", "avx2"].into_iter().collect();
        assert!(has_all_flags(&flags, &["lm", "sse2"]));
    }

    #[test]
    fn test_has_all_flags_missing() {
        let flags: HashSet<&str> = vec!["lm", "sse2"].into_iter().collect();
        assert!(!has_all_flags(&flags, &["lm", "avx"]));
    }

    #[test]
    fn test_has_all_flags_empty_required() {
        let flags: HashSet<&str> = vec!["lm", "sse2"].into_iter().collect();
        assert!(has_all_flags(&flags, &[]));
    }

    #[test]
    fn test_detect_cpu_level_from_str_v1() {
        let cpuinfo = r#"flags           : lm cmov cx8 fpu fxsr mmx syscall sse2"#;
        let result = detect_cpu_level_from_str(cpuinfo);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_detect_cpu_level_from_str_v3() {
        let cpuinfo = r#"flags           : lm cmov cx8 fpu fxsr mmx syscall sse2 cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3 avx avx2 bmi1 bmi2 f16c fma abm movbe xsave"#;
        let result = detect_cpu_level_from_str(cpuinfo);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn test_detect_cpu_level_from_str_no_flags() {
        let cpuinfo = "processor       : 0";
        let result = detect_cpu_level_from_str(cpuinfo);
        assert!(result.is_err());
    }

    #[test]
    fn test_bbr_install_status_default() {
        let status = BbrInstallStatus {
            kernel_version: "5.15.0".to_string(),
            congestion_control: "bbr".to_string(),
            reboot_required: true,
        };
        assert_eq!(status.kernel_version, "5.15.0");
        assert!(status.reboot_required);
    }

    #[test]
    fn test_bbr_runtime_info_default() {
        let info = BbrRuntimeInfo {
            uname_r: "5.15.0-xanmod1".to_string(),
            tcp_congestion_control: "bbr".to_string(),
            proc_version: "Linux version 5.15.0-xanmod1".to_string(),
            has_xanmod_kernel: true,
            has_xanmod_proc_version: true,
        };
        assert!(info.has_xanmod_kernel);
        assert!(info.has_xanmod_proc_version);
    }

    #[test]
    fn test_destruct_targets_not_empty() {
        assert!(!MaintenanceManager::DESTRUCT_TARGETS.is_empty());
        assert!(MaintenanceManager::DESTRUCT_TARGETS.contains(&"/etc/wwps"));
    }

    #[test]
    fn test_destruct_services_not_empty() {
        assert!(!MaintenanceManager::DESTRUCT_SERVICES.is_empty());
        assert!(MaintenanceManager::DESTRUCT_SERVICES.contains(&"wwps-core"));
    }

    #[tokio::test]
    async fn test_ensure_geodata_files_exist_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        tokio::fs::write(format!("{}/geoip.dat", dir_path), b"test")
            .await
            .unwrap();
        tokio::fs::write(format!("{}/geosite.dat", dir_path), b"test")
            .await
            .unwrap();

        let result = MaintenanceManager::ensure_geodata_in_dir(&dir_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ensure_geodata_missing_file_triggers_download_flag() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        tokio::fs::write(format!("{}/geoip.dat", dir_path), b"test")
            .await
            .unwrap();

        let result = MaintenanceManager::ensure_geodata_in_dir(&dir_path).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_singbox_convert_args() {
        let cmds = MaintenanceManager::singbox_convert_args(
            "/opt/wwps-box",
            "geosite",
            "cn",
            "/tmp/geosite.db",
            "/tmp/geosite-cn.json",
            "/etc/wwps/wwps-box/rule-set/geosite-cn.srs",
        );
        assert_eq!(cmds.len(), 2);
        assert_eq!(
            cmds[0],
            vec![
                "/opt/wwps-box",
                "geosite",
                "export",
                "cn",
                "-f",
                "/tmp/geosite.db",
                "-o",
                "/tmp/geosite-cn.json",
            ]
        );
        assert_eq!(
            cmds[1],
            vec![
                "/opt/wwps-box",
                "rule-set",
                "compile",
                "--output",
                "/etc/wwps/wwps-box/rule-set/geosite-cn.srs",
                "/tmp/geosite-cn.json",
            ]
        );
    }

    #[test]
    fn test_singbox_convert_args_geoip_uses_geoip_export() {
        let cmds = MaintenanceManager::singbox_convert_args(
            "/opt/wwps-box",
            "geoip",
            "cn",
            "/tmp/geoip.db",
            "/tmp/geoip-cn.json",
            "/etc/wwps/wwps-box/rule-set/geoip-cn.srs",
        );
        assert_eq!(cmds.len(), 2);
        // geoip 分类必须用 `geoip export` 子命令（写死 geosite 会静默失败）
        assert_eq!(
            cmds[0],
            vec![
                "/opt/wwps-box",
                "geoip",
                "export",
                "cn",
                "-f",
                "/tmp/geoip.db",
                "-o",
                "/tmp/geoip-cn.json",
            ]
        );
        assert_eq!(
            cmds[1],
            vec![
                "/opt/wwps-box",
                "rule-set",
                "compile",
                "--output",
                "/etc/wwps/wwps-box/rule-set/geoip-cn.srs",
                "/tmp/geoip-cn.json",
            ]
        );
    }

    #[tokio::test]
    async fn test_ensure_singbox_rule_sets_skips_existing() {
        let temp = tempfile::tempdir().unwrap();
        let rule_dir = temp.path().join("rule-set");
        tokio::fs::create_dir_all(&rule_dir).await.unwrap();
        let existing = rule_dir.join("geosite-cn.srs");
        tokio::fs::write(&existing, b"data").await.unwrap();
        // 文件已存在 → 对应分类不应出现在待下载/转换清单
        let missing = MaintenanceManager::singbox_missing_srs(rule_dir.to_str().unwrap());
        assert!(
            !missing
                .iter()
                .any(|(kind, cat)| *kind == "geosite" && *cat == "cn")
        );
        assert!(
            missing
                .iter()
                .any(|(kind, cat)| *kind == "geosite" && *cat == "private")
        );
    }
}
