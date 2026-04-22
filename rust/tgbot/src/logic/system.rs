#![allow(dead_code, unused_variables)]
use anyhow::Result;
use std::time::Duration;

use reqwest::Client;
use sysinfo::{Disks, Networks, System};
use tokio::process::Command;

pub struct SystemMonitor;

impl SystemMonitor {
    /// 获取系统完整状态报告 (formatted string)
    pub async fn get_status_report() -> Result<String> {
        let interval = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

        let (sys_data, cpu_usage) = tokio::task::spawn_blocking(move || {
            let mut sys = System::new_all();
            sys.refresh_all();
            sys.refresh_all();

            let total_mem = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
            let used_mem = sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
            let mem_percent = if total_mem > 0.0 {
                (used_mem / total_mem) * 100.0
            } else {
                0.0
            };

            let total_swap = sys.total_swap() as f64 / 1024.0 / 1024.0 / 1024.0;
            let used_swap = sys.used_swap() as f64 / 1024.0 / 1024.0 / 1024.0;

            let disks = Disks::new_with_refreshed_list();
            let total_disk: u64 = disks.list().iter().map(|d| d.total_space()).sum();
            let used_disk: u64 = disks
                .list()
                .iter()
                .map(|d| d.total_space() - d.available_space())
                .sum();
            let total_disk_gb = total_disk as f64 / 1024.0 / 1024.0 / 1024.0;
            let used_disk_gb = used_disk as f64 / 1024.0 / 1024.0 / 1024.0;
            let disk_percent = if total_disk > 0 {
                (used_disk as f64 / total_disk as f64) * 100.0
            } else {
                0.0
            };

            let networks = Networks::new_with_refreshed_list();
            let (rx_total, tx_total) = aggregate_network_traffic(&networks);
            let rx_gb = rx_total as f64 / 1024.0 / 1024.0 / 1024.0;
            let tx_gb = tx_total as f64 / 1024.0 / 1024.0 / 1024.0;

            let uptime = System::uptime();
            let days = uptime / 86400;
            let hours = (uptime % 86400) / 3600;
            let minutes = (uptime % 3600) / 60;

            (
                (
                    total_mem,
                    used_mem,
                    mem_percent,
                    total_swap,
                    used_swap,
                    total_disk_gb,
                    used_disk_gb,
                    disk_percent,
                    rx_gb,
                    tx_gb,
                    days,
                    hours,
                    minutes,
                ),
                sys.global_cpu_info().cpu_usage(),
            )
        })
        .await?;

        tokio::time::sleep(interval).await;

        let cpu_usage = tokio::task::spawn_blocking(move || {
            let mut sys = System::new();
            sys.refresh_cpu();
            sys.refresh_cpu();
            sys.global_cpu_info().cpu_usage()
        })
        .await?;

        let (
            total_mem,
            used_mem,
            mem_percent,
            total_swap,
            used_swap,
            total_disk_gb,
            used_disk_gb,
            disk_percent,
            rx_gb,
            tx_gb,
            days,
            hours,
            minutes,
        ) = sys_data;

        Ok(format!(
            "🖥 **系统状态**:\n\
            ⏱ 运行时间: {}天 {}小时 {}分\n\
            💹 CPU使用: {:.1}%\n\
            🧠 内存: {:.2} / {:.2} GB ({:.1}%)\n\
            🔁 Swap: {:.2} / {:.2} GB\n\
            💾 硬盘: {:.2} / {:.2} GB ({:.1}%)\n\
            🌐 网络流量: ⬇️ {:.2} GB | ⬆️ {:.2} GB",
            days,
            hours,
            minutes,
            cpu_usage,
            used_mem,
            total_mem,
            mem_percent,
            used_swap,
            total_swap,
            used_disk_gb,
            total_disk_gb,
            disk_percent,
            rx_gb,
            tx_gb
        ))
    }

    /// 兼容旧 API: 获取 CPU 使用率字符串
    pub async fn get_cpu_usage() -> Result<String> {
        let interval = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

        tokio::task::spawn_blocking(|| {
            let mut sys = System::new();
            sys.refresh_cpu();
        })
        .await?;

        tokio::time::sleep(interval).await;

        let cpu_usage = tokio::task::spawn_blocking(|| {
            let mut sys = System::new();
            sys.refresh_cpu();
            sys.refresh_cpu();
            sys.global_cpu_info().cpu_usage()
        })
        .await?;

        Ok(format!("{:.1}%", cpu_usage))
    }

    /// 兼容旧 API: 获取内存使用率字符串
    pub async fn get_memory_usage() -> Result<String> {
        tokio::task::spawn_blocking(|| {
            let mut sys = System::new();
            sys.refresh_memory();
            let total = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
            let used = sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
            let percent = if total > 0.0 {
                (used / total) * 100.0
            } else {
                0.0
            };
            Ok(format!("{:.2} / {:.2} GB ({:.1}%)", used, total, percent))
        })
        .await?
    }

    /// 兼容旧 API: 获取网络流量字符串
    pub async fn get_network_traffic() -> Result<String> {
        tokio::task::spawn_blocking(|| {
            let networks = Networks::new_with_refreshed_list();
            let (rx_total, tx_total) = aggregate_network_traffic(&networks);
            Ok(format!(
                "⬇️ {:.2} GB | ⬆️ {:.2} GB",
                rx_total as f64 / 1024.0 / 1024.0 / 1024.0,
                tx_total as f64 / 1024.0 / 1024.0 / 1024.0
            ))
        })
        .await?
    }

    /// 兼容旧 API: 获取负载
    pub async fn get_load_avg() -> Result<String> {
        tokio::task::spawn_blocking(|| {
            let load = System::load_average();
            Ok(format!(
                "{:.2} {:.2} {:.2}",
                load.one, load.five, load.fifteen
            ))
        })
        .await?
    }

    /// 获取核心进程状态
    pub async fn get_core_status() -> (bool, bool) {
        tokio::task::spawn_blocking(|| {
            let sys = System::new_all();
            let wwps_core = sys.processes_by_name("wwps-core".as_ref()).next().is_some();
            let wwps_box = sys.processes_by_name("wwps-box".as_ref()).next().is_some();
            (wwps_core, wwps_box)
        })
        .await
        .unwrap_or((false, false))
    }

    /// 获取公网 IP (异步)
    pub async fn get_public_ip() -> Result<String> {
        let client = Client::builder()
            .user_agent("wwps-runtime-monitor/1.0")
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new());

        let apis = [
            "https://api4.ipify.org",
            "https://ipv4.icanhazip.com",
            "https://v4.api.ipinfo.io/ip",
            "https://ipv4.myexternalip.com/raw",
        ];

        for api in apis {
            // No block_on!
            if let Ok(resp) = client.get(api).send().await {
                if !resp.status().is_success() {
                    continue;
                }
                if let Ok(text) = resp.text().await {
                    let ip = text.trim();
                    if !ip.is_empty() && ip.chars().all(|c| c.is_ascii_digit() || c == '.') {
                        return Ok(ip.to_string());
                    }
                }
            }
        }

        // 回退：本地 host 文件 (Async FS)
        if let Ok(host) = tokio::fs::read_to_string("/etc/wwps/host").await {
            let host = host.trim().to_string();
            if !host.is_empty() {
                return Ok(host);
            }
        }

        anyhow::bail!("无法获取公网 IPv4 地址")
    }

    /// 获取公网 IPv6 (异步)
    pub async fn get_public_ipv6() -> Result<String> {
        let client = Client::builder()
            .user_agent("wwps-runtime-monitor/1.0")
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new());

        let apis = [
            "https://api6.ipify.org",
            "https://ipv6.icanhazip.com",
            "https://v6.api.ipinfo.io/ip",
            "https://ipv6.myexternalip.com/raw",
        ];

        for api in apis {
            let resp = client.get(api).send().await;
            if let Ok(resp) = resp {
                if !resp.status().is_success() {
                    continue;
                }
                if let Ok(text) = resp.text().await {
                    let ip = text.trim();
                    if !ip.is_empty() && ip.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
                        return Ok(ip.to_string());
                    }
                }
            }
        }

        // 回退: 尝试从本地接口获取 (需要更多 logic，这里暂时返回 error)
        anyhow::bail!("无法获取公网 IPv6 地址")
    }

    // Check service status utilizing systemctl asynchronously
    pub async fn check_service_status(service_name: &str) -> bool {
        let output = Command::new("systemctl")
            .arg("is-active")
            .arg(service_name)
            .output()
            .await;

        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim() == "active",
            Err(_) => false,
        }
    }
}

fn aggregate_network_traffic(networks: &Networks) -> (u64, u64) {
    networks
        .list()
        .values()
        .fold((0u64, 0u64), |(rx, tx), data| {
            (rx + data.total_received(), tx + data.total_transmitted())
        })
}
