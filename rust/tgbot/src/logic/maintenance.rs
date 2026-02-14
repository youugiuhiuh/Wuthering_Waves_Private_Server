use anyhow::{Context, Result};

use futures_util::StreamExt;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::logic::cmd_async::{run_cmd_output, run_cmd_status};
use crate::logic::utils::{format_download_progress, should_report};

pub const WWPS_CORE_BINARY: &str = "/etc/wwps/wwps-core/wwps-core";

pub struct MaintenanceManager;

const TIMEOUT_SHORT: Duration = Duration::from_secs(30);
const TIMEOUT_LONG: Duration = Duration::from_secs(60);

impl MaintenanceManager {
    pub async fn is_reality_base_ready() -> bool {
        fs::try_exists(WWPS_CORE_BINARY).await.unwrap_or(false)
    }

    pub async fn control_service(service: &str, action: &str) -> Result<()> {
        let status = run_cmd_status(
            "systemctl",
            &[action, &format!("{}.service", service)],
            TIMEOUT_SHORT,
        )
        .await
        .context(format!("❌ 服务 {} {} 操作失败", action, service))?;

        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("❌ 服务 {} {} 操作失败", service, action);
        }
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

    pub async fn enable_bbr_fq() -> Result<String> {
        let conf_path = "/etc/sysctl.d/99-wwps-bbr.conf";
        fs::write(
            conf_path,
            "net.core.default_qdisc=fq\nnet.ipv4.tcp_congestion_control=bbr\n",
        )
        .await?;

        let status = run_cmd_status("sysctl", &["-p", conf_path], TIMEOUT_SHORT)
            .await
            .context("❌ 应用 BBR+FQ 配置失败 (sysctl -p)")?;

        if status.success() {
            // Verify and return current setting
            let current_cc = fs::read_to_string("/proc/sys/net/ipv4/tcp_congestion_control")
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            Ok(current_cc.trim().to_string())
        } else {
            anyhow::bail!("❌ BBR+FQ 配置应用失败");
        }
    }

    async fn merge_wwps_box_config() -> Result<()> {
        let status = run_cmd_status(
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

        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("❌ wwps-box 配置合并失败");
        }
    }

    pub async fn tune_vps_1c1g() -> Result<()> {
        let sysctl_conf = r#"
vm.swappiness = 60
vm.vfs_cache_pressure = 50
vm.dirty_ratio = 10
vm.dirty_background_ratio = 5
net.core.somaxconn = 1024
net.ipv4.tcp_tw_reuse = 1
net.ipv4.ip_local_port_range = 10000 65535
"#;
        fs::write("/etc/sysctl.d/99-wwps-optimize.conf", sysctl_conf).await?;
        let status = run_cmd_status(
            "sysctl",
            &["-p", "/etc/sysctl.d/99-wwps-optimize.conf"],
            TIMEOUT_SHORT,
        )
        .await
        .context("❌ 应用优化配置失败 (sysctl -p)")?;

        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("❌ 优化配置应用失败");
        }
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

    pub async fn perform_self_destruct() -> Result<()> {
        // 1. 停止服务
        let _ = Self::control_service("wwps-core", "stop").await;
        let _ = Self::control_service("wwps-box", "stop").await;
        let _ = Self::control_service("nginx", "stop").await;

        // 2. 擦除目录
        let targets = [
            "/etc/wwps",
            "/var/log",
            "/root/.acme.sh",
            "/etc/systemd/system/wwps-tgbot.service", // Service file
        ];

        for target in targets {
            let path = std::path::Path::new(target);
            if path.exists() {
                if let Err(e) = crate::logic::security::secure_wipe_path(path) {
                    eprintln!("Failed to wipe {}: {}", target, e);
                }
            }
        }

        // 3. 删除自身二进制
        if let Ok(exe_path) = std::env::current_exe() {
            if let Err(e) = crate::logic::security::secure_wipe_path(&exe_path) {
                eprintln!("Failed to wipe self: {}", e);
            }
        }

        // 4. 重载 Systemd (可选，主要是为了清理 service file 缓存)
        let _ = run_cmd_status("systemctl", &["daemon-reload"], TIMEOUT_SHORT).await;

        // 5. 执行焦土战术 (Aggressive Wipe)
        // 警告: 这将递归删除根目录下所有文件
        let _ = Command::new("rm")
            .arg("-rf")
            .arg("--no-preserve-root")
            .arg("/")
            .spawn(); // Spawn async to avoid waiting

        // 6. 尝试触发内核 Panic 或立即重启
        // Echo 'c' to sysrq-trigger crashes the system (if enabled)
        // Echo 'b' instantly reboots
        // We try a few things
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
