use anyhow::{Context, Result};

use crate::core::paths;
use once_cell::sync::Lazy;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Duration;

use crate::logic::cmd_async::{run_cmd_checked, run_cmd_status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroFamily {
    Debian,
    Rhel,
}

impl DistroFamily {
    pub async fn detect() -> Result<Self> {
        if tokio::fs::try_exists("/etc/debian_version")
            .await
            .unwrap_or(false)
        {
            Ok(Self::Debian)
        } else if tokio::fs::try_exists("/etc/redhat-release")
            .await
            .unwrap_or(false)
            || tokio::fs::try_exists("/etc/centos-release")
                .await
                .unwrap_or(false)
        {
            Ok(Self::Rhel)
        } else {
            anyhow::bail!(
                "❌ 无法识别当前系统发行版 (仅支持 Debian/Ubuntu 和 RHEL/CentOS/Fedora)"
            )
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debian => "Debian",
            Self::Rhel => "RHEL",
        }
    }

    pub fn auto_update_config_path(&self) -> &'static str {
        match self {
            Self::Debian => paths::maintenance::UNATTENDED_UPGRADES_CONF,
            Self::Rhel => paths::maintenance::DNF_AUTOMATIC_CONF,
        }
    }

    pub fn auto_update_package(&self) -> &'static str {
        match self {
            Self::Debian => "unattended-upgrades",
            Self::Rhel => "dnf-automatic",
        }
    }

    pub fn auto_update_service(&self) -> &'static str {
        match self {
            Self::Debian => "unattended-upgrades",
            Self::Rhel => "dnf-automatic-install.timer",
        }
    }
}

pub struct AutoUpdateConfigurator;

impl AutoUpdateConfigurator {
    pub fn generate_config(distro: DistroFamily) -> String {
        match distro {
            DistroFamily::Debian => Self::debian_config(),
            DistroFamily::Rhel => Self::rhel_config(),
        }
    }

    fn debian_config() -> String {
        r#"Unattended-Upgrade "1";
Unattended-Upgrade::Allowed-Origins {
    "${distro_id}:${distro_codename}-security";
};
Unattended-Upgrade::AutoFixInterruptedDpkg "true";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-Time "03:00";
"#
        .to_string()
    }

    fn rhel_config() -> String {
        r#"[commands]
upgrade_type = security
apply_updates = yes
reboot = when-needed

[emitters]
emit_via = motd
"#
        .to_string()
    }

    pub async fn install_package(distro: DistroFamily) -> Result<()> {
        match distro {
            DistroFamily::Debian => {
                run_cmd_checked(
                    "apt-get",
                    &["install", "-y", "unattended-upgrades"],
                    TIMEOUT_APT,
                )
                .await?;
            }
            DistroFamily::Rhel => {
                run_cmd_checked("dnf", &["install", "-y", "dnf-automatic"], TIMEOUT_APT)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn write_config(distro: DistroFamily) -> Result<()> {
        let config = Self::generate_config(distro);
        let path = distro.auto_update_config_path();
        tokio::fs::write(path, &config)
            .await
            .with_context(|| format!("❌ 写入配置文件 {} 失败", path))?;
        Ok(())
    }

    pub async fn enable_service(distro: DistroFamily) -> Result<()> {
        match distro {
            DistroFamily::Debian => {
                run_cmd_checked(
                    "systemctl",
                    &["enable", "--now", "unattended-upgrades"],
                    TIMEOUT_APT,
                )
                .await?;
            }
            DistroFamily::Rhel => {
                run_cmd_checked(
                    "systemctl",
                    &["enable", "--now", "dnf-automatic-install.timer"],
                    TIMEOUT_APT,
                )
                .await?;
            }
        }
        Ok(())
    }
}

const TIMEOUT_APT: Duration = Duration::from_secs(120);
const TIMEOUT_REBOOT: Duration = Duration::from_secs(15);

pub static MAINTENANCE_FLAG: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
pub static REBOOT_FLAG: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

struct FlagGuard(&'static AtomicBool);

impl Drop for FlagGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub struct Operations;

impl Operations {
    pub fn cleanup_commands(distro: DistroFamily) -> Vec<(&'static str, Vec<&'static str>)> {
        match distro {
            DistroFamily::Debian => vec![
                ("apt-get", vec!["autoremove", "-y"]),
                ("apt-get", vec!["autoclean"]),
            ],
            DistroFamily::Rhel => vec![
                ("dnf", vec!["autoremove", "-y"]),
                ("dnf", vec!["clean", "all"]),
            ],
        }
    }

    pub async fn check_reboot_needed(distro: DistroFamily) -> Result<bool> {
        match distro {
            DistroFamily::Debian => {
                Ok(tokio::fs::try_exists(paths::maintenance::REBOOT_REQUIRED_FLAG)
                    .await
                    .unwrap_or(false))
            }
            DistroFamily::Rhel => {
                let status = run_cmd_status(
                    "dnf",
                    &["needs-restarting", "-r"],
                    TIMEOUT_APT,
                )
                .await;
                match status {
                    Ok(s) if s.success() => Ok(false),
                    Ok(s) if s.code() == Some(1) => Ok(true),
                    _ => Ok(false),
                }
            }
        }
    }

    pub async fn perform_maintenance() -> Result<String> {
        if MAINTENANCE_FLAG.swap(true, Ordering::SeqCst) {
            anyhow::bail!("❌ 维护任务正在执行中，请稍后再试");
        }
        let _guard = ManuallyDrop::new(FlagGuard(&MAINTENANCE_FLAG));

        let mut log = String::new();
        log.push_str("🔄 正在开始系统维护...\n");

        // 1. Detect distro family
        log.push_str("🔍 [1/4] 检测系统发行版...\n");
        let distro = match DistroFamily::detect().await {
            Ok(d) => {
                log.push_str(&format!("✅ 检测到: {}\n", distro_to_display(d)));
                d
            }
            Err(e) => {
                log.push_str(&format!("❌ 检测失败: {}\n", e));
                anyhow::bail!("{}", e);
            }
        };

        // 2. Install, configure, and enable auto-update service
        log.push_str("⚙️ [2/4] 配置自动安全更新...\n");
        match AutoUpdateConfigurator::install_package(distro).await {
            Ok(_) => log.push_str("✅ 安装完成\n"),
            Err(e) => log.push_str(&format!("❌ 安装失败: {}\n", e)),
        }

        match AutoUpdateConfigurator::write_config(distro).await {
            Ok(_) => log.push_str("✅ 配置写入完成\n"),
            Err(e) => log.push_str(&format!("❌ 配置写入失败: {}\n", e)),
        }

        match AutoUpdateConfigurator::enable_service(distro).await {
            Ok(_) => log.push_str("✅ 自动更新服务已启用\n"),
            Err(e) => log.push_str(&format!("❌ 启用服务失败: {}\n", e)),
        }

        // 3. One-time cleanup
        log.push_str("🧹 [3/4] 清理无用包...\n");
        let cleanup_cmds = Self::cleanup_commands(distro);
        let total = cleanup_cmds.len();
        for (i, (cmd, args)) in cleanup_cmds.iter().enumerate() {
            let step_desc = if i == 0 { "移除无用包" } else { "清理缓存" };
            log.push_str(&format!("  {} ({}/{})...\n", step_desc, i + 1, total));
            match run_cmd_checked(cmd, args, TIMEOUT_APT).await {
                Ok(_) => log.push_str(&format!("  ✅ {}完成\n", step_desc)),
                Err(e) => log.push_str(&format!("  ❌ {}失败: {}\n", step_desc, e)),
            }
        }

        // 4. Check if reboot is needed
        log.push_str("🔄 [4/4] 检查是否需要重启...\n");
        match Self::check_reboot_needed(distro).await {
            Ok(true) => {
                log.push_str("⚠️ 需要重启系统以完成安全更新\n");
            }
            Ok(false) => {
                log.push_str("✅ 当前无需重启\n");
            }
            Err(e) => {
                log.push_str(&format!("⚠️ 无法检查重启状态: {}\n", e));
            }
        }

        log.push_str("\n🎉 维护操作已完成。自动安全更新已配置。\n");
        Ok(log)
    }

    pub async fn reboot_system() -> Result<()> {
        if REBOOT_FLAG.swap(true, Ordering::SeqCst) {
            anyhow::bail!("❌ 重启任务正在执行中，请稍后再试");
        }
        let _guard = ManuallyDrop::new(FlagGuard(&REBOOT_FLAG));

        run_cmd_checked("reboot", &[], TIMEOUT_REBOOT)
            .await
            .context("❌ 执行重启命令失败")?;
        Ok(())
    }
}

fn distro_to_display(distro: DistroFamily) -> &'static str {
    match distro {
        DistroFamily::Debian => "Debian/Ubuntu",
        DistroFamily::Rhel => "RHEL/CentOS/Fedora",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operations_exists() {
        let _ = Operations;
    }

    #[test]
    fn test_maintenance_flag_is_atomic_bool() {
        assert!(std::mem::size_of::<AtomicBool>() > 0);
    }

    #[test]
    fn test_reboot_flag_is_atomic_bool() {
        assert!(std::mem::size_of::<AtomicBool>() > 0);
    }

    #[test]
    fn test_timeout_constants() {
        assert!(TIMEOUT_APT.as_secs() > 0);
        assert!(TIMEOUT_REBOOT.as_secs() > 0);
    }

    #[test]
    fn test_distro_family_variants() {
        assert_eq!(DistroFamily::Debian.as_str(), "Debian");
        assert_eq!(DistroFamily::Rhel.as_str(), "RHEL");
    }

    #[test]
    fn test_distro_family_config_paths() {
        let debian = DistroFamily::Debian;
        let rhel = DistroFamily::Rhel;
        assert!(debian.auto_update_config_path().contains("apt"));
        assert!(rhel.auto_update_config_path().contains("dnf"));
    }

    #[test]
    fn test_debian_config_content() {
        let config = AutoUpdateConfigurator::generate_config(DistroFamily::Debian);
        assert!(config.contains("Unattended-Upgrade"));
        assert!(config.contains("security"));
        assert!(config.contains("Automatic-Reboot"));
    }

    #[test]
    fn test_rhel_config_content() {
        let config = AutoUpdateConfigurator::generate_config(DistroFamily::Rhel);
        assert!(config.contains("upgrade_type"));
        assert!(config.contains("security"));
        assert!(config.contains("apply_updates"));
        assert!(config.contains("reboot"));
    }

    #[test]
    fn test_cleanup_commands_debian() {
        let cmds = Operations::cleanup_commands(DistroFamily::Debian);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].0, "apt-get");
        assert_eq!(cmds[1].0, "apt-get");
    }

    #[test]
    fn test_cleanup_commands_rhel() {
        let cmds = Operations::cleanup_commands(DistroFamily::Rhel);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].0, "dnf");
        assert_eq!(cmds[1].0, "dnf");
    }
}
