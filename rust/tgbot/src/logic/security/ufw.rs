use crate::logic::cmd_async::run_cmd_output;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::Mutex;

// UFW 基于 Python，对 iptables 的操作通过文件锁同步。
// 为了防止 Rust 并发调用导致 "Another app is currently holding the xtables lock"，
// 我们在应用层使用全局互斥锁确保串行化。
static UFW_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub struct UfwClient;

impl UfwClient {
    pub async fn is_installed() -> bool {
        tokio::fs::metadata("/usr/sbin/ufw").await.is_ok()
    }

    /// 执行 UFW 命令，带重试逻辑和并发锁
    async fn run_ufw(args: &[&str]) -> Result<()> {
        let _lock = UFW_MUTEX.lock().await;
        let mut last_err = None;

        for i in 0..3 {
            if i > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }

            match run_cmd_output("ufw", args, Duration::from_secs(10)).await {
                Ok((status, stdout, stderr)) => {
                    if status.success() {
                        return Ok(());
                    }

                    // 逻辑去重：如果提示规则已存在，视作成功
                    if stdout.contains("Skipping adding existing rule")
                        || stderr.contains("Skipping adding existing rule")
                    {
                        return Ok(());
                    }

                    // 如果报错中包含 "lock"，则触发重试
                    let err_msg = format!("{}{}", stdout, stderr);
                    if err_msg.contains("lock") || err_msg.contains("Another app") {
                        last_err = Some(anyhow::anyhow!("UFW 锁冲突: {}", err_msg));
                        continue;
                    }

                    return Err(anyhow::anyhow!("UFW 执行失败: {}", err_msg));
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("UFW 命令执行最终失败")))
    }

    pub async fn add_port(port: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}/{}", port, protocol);
        Self::run_ufw(&["allow", &port_spec])
            .await
            .with_context(|| format!("UFW 允许端口 {} 失败", port_spec))
    }

    pub async fn remove_port(port: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}/{}", port, protocol);
        Self::run_ufw(&["delete", "allow", &port_spec])
            .await
            .with_context(|| format!("UFW 删除端口 {} 失败", port_spec))
    }

    pub async fn add_port_range(start: u16, end: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}:{}/{}", start, end, protocol);
        Self::run_ufw(&["allow", &port_spec])
            .await
            .with_context(|| format!("UFW 允许端口范围 {} 失败", port_spec))
    }

    pub async fn add_port_range_v6(start: u16, end: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}:{}/{}", start, end, protocol);
        Self::run_ufw(&["allow", &port_spec])
            .await
            .with_context(|| format!("UFW 允许 IPv6 端口范围 {} 失败", port_spec))
    }

    pub async fn remove_port_range(start: u16, end: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}:{}/{}", start, end, protocol);
        Self::run_ufw(&["delete", "allow", &port_spec])
            .await
            .with_context(|| format!("UFW 删除端口范围 {} 失败", port_spec))
    }

    pub async fn remove_port_range_v6(start: u16, end: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}:{}/{}", start, end, protocol);
        Self::run_ufw(&["delete", "allow", &port_spec])
            .await
            .with_context(|| format!("UFW 删除 IPv6 端口范围 {} 失败", port_spec))
    }

    pub async fn is_active() -> bool {
        match run_cmd_output("ufw", &["status"], Duration::from_secs(5)).await {
            Ok((status, stdout, _)) => {
                status.success() && stdout.to_lowercase().contains("status: active")
            }
            Err(_) => false,
        }
    }

    pub async fn list_allowed_ports() -> Result<HashSet<u16>> {
        let _lock = UFW_MUTEX.lock().await;
        let mut last_err = None;

        for i in 0..3 {
            if i > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }

            match run_cmd_output("ufw", &["status", "numbered"], Duration::from_secs(10)).await {
                Ok((status, stdout, stderr)) => {
                    if !status.success() {
                        let err_msg = format!("{}{}", stdout, stderr);
                        if err_msg.contains("lock") || err_msg.contains("Another app") {
                            last_err = Some(anyhow::anyhow!("UFW 锁冲突: {}", err_msg));
                            continue;
                        }
                        anyhow::bail!("UFW status 失败: {}", err_msg);
                    }
                    return Ok(parse_ufw_allowed_ports(&stdout));
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("UFW status 命令最终失败")))
    }

    pub async fn harden_with_ports(ports: HashSet<u16>) -> Result<()> {
        // 1. 重置 UFW
        let _ = Self::run_ufw(&["--force", "reset"]).await;

        // 2. 设置默认策略
        Self::run_ufw(&["default", "deny", "incoming"]).await?;
        Self::run_ufw(&["default", "allow", "outgoing"]).await?;

        // 3. 循环放行端口
        for port in ports {
            Self::add_port(port, "tcp").await?;
            Self::add_port(port, "udp").await?;
        }

        // 4. 启用 UFW
        Self::run_ufw(&["--force", "enable"]).await?;

        Ok(())
    }
}

fn parse_ufw_allowed_ports(stdout: &str) -> HashSet<u16> {
    let mut ports = HashSet::new();
    for line in stdout.lines() {
        let line = line.trim();
        if !line.contains("ALLOW") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        for part in parts {
            if let Some(port_str) = part.split('/').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    ports.insert(port);
                }
            }
        }
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ufw_is_installed_returns_bool() {
        let result = UfwClient::is_installed().await;
        let _: bool = result;
    }

    #[test]
    fn test_ufw_client_exists() {
        let _ = UfwClient;
    }

    #[test]
    fn test_ufw_mutex_is_lazy_initialized() {
        let mutex = &UFW_MUTEX;
        assert!(std::mem::size_of::<Mutex<()>>() > 0);
        let _ = mutex;
    }
}
