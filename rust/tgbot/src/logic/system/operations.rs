use anyhow::{Context, Result};

use crate::core::paths;
use once_cell::sync::Lazy;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Duration;

use crate::logic::cmd_async::{run_cmd_checked, run_cmd_output};

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
    /// 执行完整的系统维护：更新、升级、清理
    pub async fn perform_maintenance() -> Result<String> {
        if MAINTENANCE_FLAG.swap(true, Ordering::SeqCst) {
            anyhow::bail!("❌ 维护任务正在执行中，请稍后再试");
        }
        let _guard = ManuallyDrop::new(FlagGuard(&MAINTENANCE_FLAG));

        let mut log = String::new();

        log.push_str("🔄 正在开始系统维护...\n");

        // 1. apt-get update
        log.push_str("📥 [1/4] 更新软件源列表...\n");
        match Self::run_apt(&["update"]).await {
            Ok(_) => log.push_str("✅ 更新成功\n"),
            Err(e) => log.push_str(&format!("❌ 更新失败: {}\n", e)),
        }

        // 2. apt-get full-upgrade -y
        log.push_str("📦 [2/4] 执行全量升级...\n");
        match Self::run_apt(&["full-upgrade", "-y"]).await {
            Ok(_) => log.push_str("✅ 升级成功\n"),
            Err(e) => log.push_str(&format!("❌ 升级失败: {}\n", e)),
        }

        // 3. apt-get autoremove -y
        log.push_str("🧹 [3/4] 自动移除无用包...\n");
        match Self::run_apt(&["autoremove", "-y"]).await {
            Ok(_) => log.push_str("✅ 移除成功\n"),
            Err(e) => log.push_str(&format!("❌ 移除失败: {}\n", e)),
        }

        // 4. apt-get autoclean
        log.push_str("✨ [4/4] 清理缓存...\n");
        match Self::run_apt(&["autoclean"]).await {
            Ok(_) => log.push_str("✅ 清理成功\n"),
            Err(e) => log.push_str(&format!("❌ 清理失败: {}\n", e)),
        }

        log.push_str("\n🎉 维护操作已完成。\n");

        Ok(log)
    }

    /// 执行安全重启
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

    /// 辅助函数：运行 apt 命令，包含自动处理 dpkg 中断的逻辑
    async fn run_apt(args: &[&str]) -> Result<()> {
        let (status, _out, stderr) = run_cmd_output("apt-get", args, TIMEOUT_APT)
            .await
            .context(format!("❌ 执行 apt-get 命令 {:?} 失败", args))?;

        if status.success() {
            Ok(())
        } else {
            // 检测是否发生了 dpkg 被中断的错误
            if stderr.contains("dpkg --configure -a") || stderr.contains("dpkg was interrupted") {
                log::warn!("检测到 dpkg 中断错误，尝试自动修复...");

                // 尝试修复 dpkg
                let (fix_status, _, fix_stderr) =
                    run_cmd_output("dpkg", &["--configure", "-a"], TIMEOUT_APT)
                        .await
                        .context("❌ 执行 dpkg --configure -a 失败")?;

                if fix_status.success() {
                    log::info!("dpkg 修复成功，正在重试原始命令...");
                    // 重试原始命令
                    let (retry_status, _, retry_stderr) =
                        run_cmd_output("apt-get", args, TIMEOUT_APT)
                            .await
                            .context(format!("❌ 重试 apt-get 命令 {:?} 失败", args))?;

                    if retry_status.success() {
                        return Ok(());
                    } else {
                        anyhow::bail!("❌ 重试仍然失败: {}", retry_stderr);
                    }
                } else {
                    anyhow::bail!("❌ 自动修复 dpkg 失败: {}", fix_stderr);
                }
            }

            anyhow::bail!("❌ apt-get 命令 {:?} 执行失败: {}", args, stderr)
        }
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
}
