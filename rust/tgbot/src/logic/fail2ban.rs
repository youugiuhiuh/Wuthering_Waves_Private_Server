use crate::logic::cmd_async::run_cmd_status;
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::fs;

pub struct Fail2BanManager;

impl Fail2BanManager {
    /// 自动安装并配置 Fail2Ban
    pub async fn setup() -> Result<()> {
        // 1. 安装 Fail2Ban
        Self::install().await?;

        // 2. 检测环境并生成配置
        Self::apply_config().await?;

        // 3. 启动并启用服务
        Self::restart_service().await?;

        Ok(())
    }

    async fn install() -> Result<()> {
        if fs::try_exists("/usr/bin/fail2ban-client")
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }

        // 简单的包管理调用，通常在 Debian/Ubuntu 环境下
        // 实际项目中可能有更复杂的 Installer 抽象，这里参考已有代码
        let _ = run_cmd_status("apt-get", &["update"], Duration::from_secs(60)).await;
        run_cmd_status(
            "apt-get",
            &["install", "-y", "fail2ban"],
            Duration::from_secs(120),
        )
        .await
        .context("安装 Fail2Ban 失败")?;

        Ok(())
    }

    async fn apply_config() -> Result<()> {
        let firewall_type = Self::detect_firewall().await;
        let action = match firewall_type.as_str() {
            "ufw" => {
                if fs::try_exists("/etc/fail2ban/action.d/ufw-allports.conf")
                    .await
                    .unwrap_or(false)
                {
                    "ufw-allports"
                } else {
                    "ufw"
                }
            }
            "firewalld" => "firewallcmd-ipset",
            _ => "iptables-allports",
        };

        let config = format!(
            r#"[DEFAULT]
banaction = {}
backend = systemd
bantime = 1h
findtime = 10m
maxretry = 3

[sshd]
enabled = true
port = ssh
logpath = /var/log/auth.log
bantime.increment = true
bantime.factor = 2
bantime.max = 1w
"#,
            action
        );

        fs::write("/etc/fail2ban/jail.local", config)
            .await
            .context("写入 Fail2Ban 配置文件失败")?;

        Ok(())
    }

    async fn detect_firewall() -> String {
        if fs::try_exists("/usr/sbin/ufw").await.unwrap_or(false) {
            "ufw".to_string()
        } else if fs::try_exists("/usr/sbin/firewalld").await.unwrap_or(false) {
            "firewalld".to_string()
        } else {
            "none".to_string()
        }
    }

    async fn restart_service() -> Result<()> {
        let _ = run_cmd_status(
            "systemctl",
            &["enable", "fail2ban"],
            Duration::from_secs(30),
        )
        .await;
        run_cmd_status(
            "systemctl",
            &["restart", "fail2ban"],
            Duration::from_secs(30),
        )
        .await
        .context("启动 Fail2Ban 服务失败")?;
        Ok(())
    }
}
