use crate::logic::firewalld::FirewalldClient;
use crate::logic::ufw::UfwClient;
use anyhow::{Result, bail};
use std::collections::HashSet;

#[derive(Debug, PartialEq)]
pub enum FirewallBackend {
    Firewalld,
    Ufw,
}

pub struct FirewallManager;

impl FirewallManager {
    pub async fn detect_backend() -> Option<FirewallBackend> {
        // 1. 状态优先：检查活跃的服务

        // 检查 Firewalld 是否活跃
        if let Ok((status, _, _)) = crate::logic::cmd_async::run_cmd_output(
            "systemctl",
            &["is-active", "firewalld"],
            std::time::Duration::from_secs(2),
        )
        .await
        {
            if status.success() {
                return Some(FirewallBackend::Firewalld);
            }
        }

        // 检查 UFW 是否活跃
        if let Ok((status, stdout, _)) = crate::logic::cmd_async::run_cmd_output(
            "ufw",
            &["status"],
            std::time::Duration::from_secs(2),
        )
        .await
        {
            if status.success() && stdout.contains("active") {
                return Some(FirewallBackend::Ufw);
            }
        }

        // 2. 启发式回退：基于二进制存在性和操作系统类型

        // 优先检测 UFW (通常在 Debian/Ubuntu 上预装或首选)
        if UfwClient::is_installed().await {
            return Some(FirewallBackend::Ufw);
        }

        // 其次检测 Firewalld
        if tokio::fs::metadata("/usr/sbin/firewalld").await.is_ok() {
            return Some(FirewallBackend::Firewalld);
        }

        None
    }

    pub async fn add_port(port: u16) -> Result<()> {
        match Self::detect_backend().await {
            Some(FirewallBackend::Ufw) => UfwClient::add_port(port, "tcp").await?,
            Some(FirewallBackend::Firewalld) => {
                FirewalldClient::add_port(port, "tcp").await?;
                FirewalldClient::add_port(port, "udp").await?;
            }
            None => bail!("未检测到支持的防火墙后端 (ufw 或 firewalld)"),
        }
        Ok(())
    }

    pub async fn harden_with_ports(ports: HashSet<u16>) -> Result<()> {
        match Self::detect_backend().await {
            Some(FirewallBackend::Ufw) => UfwClient::harden_with_ports(ports).await,
            Some(FirewallBackend::Firewalld) => FirewalldClient::harden_with_ports(ports).await,
            None => bail!("未检测到支持的防火墙后端 (ufw 或 firewalld)"),
        }
    }
}
