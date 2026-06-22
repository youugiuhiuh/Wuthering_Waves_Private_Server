use anyhow::{Context, Result};

use crate::core::paths;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Duration;

use crate::core::cmd_async::{run_cmd_checked, run_cmd_status};

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
            anyhow::bail!("❌ 无法识别当前系统发行版 (仅支持 Debian/Ubuntu 和 RHEL/CentOS/Fedora)")
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

    pub fn periodic_config_path(&self) -> Option<&'static str> {
        match self {
            Self::Debian => Some(paths::maintenance::AUTO_UPGRADES_PERIODIC_CONF),
            Self::Rhel => None,
        }
    }

    pub fn needrestart_conf_path(&self) -> &'static str {
        paths::maintenance::NEEDRESTART_CONF
    }

    pub fn supplementary_packages(&self) -> &'static [&'static str] {
        match self {
            Self::Debian => &["needrestart"],
            Self::Rhel => &["needrestart"],
        }
    }
}

pub struct AutoUpdateConfigurator;

impl AutoUpdateConfigurator {
    pub fn generate_config(distro: DistroFamily, reboot_time: &str) -> String {
        match distro {
            DistroFamily::Debian => Self::debian_config(reboot_time),
            DistroFamily::Rhel => Self::rhel_config(),
        }
    }

    fn debian_config(reboot_time: &str) -> String {
        format!(
            r#"Unattended-Upgrade::Allowed-Origins {{
    "${{distro_id}}:${{distro_codename}}-security";
    "${{distro_id}}:stable-security";
}};
Unattended-Upgrade::AutoFixInterruptedDpkg "true";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-Time "{}";
"#,
            reboot_time
        )
    }

    fn rhel_config() -> String {
        r#"[commands]
upgrade_type = security
download_updates = yes
apply_updates = yes
reboot = when-needed

[emitters]
emit_via = motd
"#
        .to_string()
    }

    pub fn apt_daily_timer_override() -> String {
        r#"[Timer]
OnCalendar=daily
RandomizedDelaySec=4h
"#
        .to_string()
    }

    pub fn apt_daily_upgrade_timer_override() -> String {
        r#"[Timer]
OnCalendar=daily
RandomizedDelaySec=4h

[Unit]
After=apt-daily.service
"#
        .to_string()
    }

    pub(crate) fn debian_periodic_config() -> String {
        r#"APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
"#
        .to_string()
    }

    pub(crate) fn needrestart_config() -> String {
        r#"$nrconf{restart} = 'a';
"#
        .to_string()
    }

    pub async fn install_package(distro: DistroFamily) -> Result<()> {
        let package = distro.auto_update_package();
        let package_manager = match distro {
            DistroFamily::Debian => "apt-get",
            DistroFamily::Rhel => "dnf",
        };
        run_cmd_checked(package_manager, &["install", "-y", package], TIMEOUT_APT).await?;
        Ok(())
    }

    pub async fn write_config(distro: DistroFamily, reboot_time: &str) -> Result<()> {
        let config = Self::generate_config(distro, reboot_time);
        let path = distro.auto_update_config_path();
        tokio::fs::write(path, &config)
            .await
            .with_context(|| format!("❌ 写入配置文件 {} 失败", path))?;
        Ok(())
    }

    pub async fn enable_service(distro: DistroFamily) -> Result<()> {
        let service = distro.auto_update_service();
        run_cmd_checked("systemctl", &["enable", "--now", service], TIMEOUT_APT).await?;

        match distro {
            DistroFamily::Debian => {
                run_cmd_checked(
                    "systemctl",
                    &["enable", "--now", "apt-daily-upgrade.timer"],
                    TIMEOUT_APT,
                )
                .await?;
            }
            DistroFamily::Rhel => {}
        }

        Ok(())
    }

    pub async fn install_supplementary_packages(
        distro: DistroFamily,
    ) -> Vec<(&'static str, Result<()>)> {
        let package_manager = match distro {
            DistroFamily::Debian => "apt-get",
            DistroFamily::Rhel => "dnf",
        };

        let pkgs = distro.supplementary_packages();
        let mut results = Vec::new();

        for &pkg in pkgs {
            let result = run_cmd_checked(package_manager, &["install", "-y", pkg], TIMEOUT_APT)
                .await
                .map(|_| ());
            results.push((pkg, result));
        }

        results
    }

    pub async fn write_periodic_config(distro: DistroFamily) -> Result<()> {
        match distro {
            DistroFamily::Debian => {
                let path = distro.periodic_config_path().unwrap();
                if let Some(parent) = std::path::Path::new(path).parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
                }
                tokio::fs::write(path, Self::debian_periodic_config())
                    .await
                    .with_context(|| format!("写入 APT Periodic 配置 {} 失败", path))?;
                Ok(())
            }
            DistroFamily::Rhel => Ok(()),
        }
    }

    pub async fn configure_needrestart(distro: DistroFamily) -> Result<()> {
        let conf_path = distro.needrestart_conf_path();
        if let Some(parent) = std::path::Path::new(conf_path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("创建 needrestart 配置目录 {} 失败", parent.display()))?;
        }

        tokio::fs::write(conf_path, Self::needrestart_config())
            .await
            .with_context(|| format!("写入 needrestart 配置 {} 失败", conf_path))?;

        Ok(())
    }
}

const TIMEOUT_APT: Duration = Duration::from_secs(120);
const TIMEOUT_REBOOT: Duration = Duration::from_secs(15);
const TIMEOUT_SHORT: Duration = Duration::from_secs(10);

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
            DistroFamily::Debian => Ok(tokio::fs::try_exists(
                paths::maintenance::REBOOT_REQUIRED_FLAG,
            )
            .await
            .unwrap_or(false)),
            DistroFamily::Rhel => {
                let status = run_cmd_status("dnf", &["needs-restarting", "-r"], TIMEOUT_APT).await;
                match status {
                    Ok(s) if s.success() => Ok(false),
                    Ok(s) if s.code() == Some(1) => Ok(true),
                    _ => Ok(false),
                }
            }
        }
    }

    pub async fn perform_maintenance() -> Result<String> {
        Self::perform_maintenance_with_reboot_time("03:00").await
    }

    pub async fn perform_maintenance_with_reboot_time(reboot_time: &str) -> Result<String> {
        if MAINTENANCE_FLAG.swap(true, Ordering::SeqCst) {
            anyhow::bail!("❌ 维护任务正在执行中，请稍后再试");
        }
        let _guard = FlagGuard(&MAINTENANCE_FLAG);

        let mut log = String::new();
        log.push_str("🔄 正在开始系统维护...\n");

        log.push_str("🔍 [1/7] 检测系统发行版...\n");
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

        log.push_str("📦 [2/7] 安装自动更新包...\n");
        match AutoUpdateConfigurator::install_package(distro).await {
            Ok(_) => log.push_str("✅ 自动更新包安装完成\n"),
            Err(e) => {
                log.push_str(&format!("❌ 安装失败: {}\n", e));
                anyhow::bail!("安装自动更新包失败: {}", e);
            }
        };

        log.push_str("📦 [3/7] 安装辅助包 (needrestart)...\n");
        let supp_results = AutoUpdateConfigurator::install_supplementary_packages(distro).await;
        for (pkg, result) in &supp_results {
            match result {
                Ok(_) => log.push_str(&format!("  ✅ {} 安装完成\n", pkg)),
                Err(e) => log.push_str(&format!("  ⚠️ {} 安装失败 (非致命): {}\n", pkg, e)),
            }
        }

        log.push_str("📝 [4/7] 写入自动更新配置...\n");
        match AutoUpdateConfigurator::write_config(distro, reboot_time).await {
            Ok(_) => log.push_str("✅ 主配置写入完成\n"),
            Err(e) => log.push_str(&format!("❌ 主配置写入失败: {}\n", e)),
        }

        log.push_str("📝 [5/7] 写入辅助配置...\n");
        match AutoUpdateConfigurator::write_periodic_config(distro).await {
            Ok(_) => log.push_str("✅ APT Periodic 配置写入完成\n"),
            Err(e) => log.push_str(&format!("⚠️ APT Periodic 配置写入失败: {}\n", e)),
        }

        let needrestart_installed = supp_results.iter().any(|(_, r)| r.is_ok());
        if needrestart_installed {
            match AutoUpdateConfigurator::configure_needrestart(distro).await {
                Ok(_) => log.push_str("✅ needrestart 自动重启配置写入完成\n"),
                Err(e) => log.push_str(&format!("⚠️ needrestart 配置写入失败: {}\n", e)),
            }
        } else {
            log.push_str("⚠️ needrestart 未安装，跳过自动重启配置\n");
        }

        log.push_str("⚡ [6/7] 启用自动更新服务...\n");
        match AutoUpdateConfigurator::enable_service(distro).await {
            Ok(_) => log.push_str("✅ 自动更新服务已启用\n"),
            Err(e) => log.push_str(&format!("❌ 启用服务失败: {}\n", e)),
        }

        log.push_str("🧹 [7/7] 清理与检查...\n");
        let cleanup_cmds = Self::cleanup_commands(distro);
        for (i, (cmd, args)) in cleanup_cmds.iter().enumerate() {
            let step_desc = if i == 0 {
                "移除无用包"
            } else {
                "清理缓存"
            };
            match run_cmd_checked(cmd, args, TIMEOUT_APT).await {
                Ok(_) => log.push_str(&format!("  ✅ {}完成\n", step_desc)),
                Err(e) => log.push_str(&format!("  ⚠️ {}失败: {}\n", step_desc, e)),
            }
        }

        match Self::check_reboot_needed(distro).await {
            Ok(true) => log.push_str("⚠️ 需要重启系统以完成安全更新\n"),
            Ok(false) => log.push_str("✅ 当前无需重启\n"),
            Err(e) => log.push_str(&format!("⚠️ 无法检查重启状态: {}\n", e)),
        }

        log.push_str("\n🎉 维护操作已完成。自动安全更新已配置。\n");
        Ok(log)
    }

    pub const DEFAULT_REBOOT_TIME: &str = "05:00";

    pub async fn perform_security_update_task() -> Result<()> {
        let distro = DistroFamily::detect().await?;
        if !tokio::fs::try_exists(distro.auto_update_config_path())
            .await
            .unwrap_or(false)
        {
            Self::perform_maintenance_with_reboot_time(Self::DEFAULT_REBOOT_TIME).await?;
        }
        Ok(())
    }

    pub async fn set_apt_daily_timer() -> Result<()> {
        let upgrade_override_dir = "/etc/systemd/system/apt-daily-upgrade.timer.d";
        tokio::fs::create_dir_all(upgrade_override_dir)
            .await
            .context("创建 apt-daily-upgrade.timer.d 目录失败")?;

        let upgrade_content = AutoUpdateConfigurator::apt_daily_upgrade_timer_override();
        tokio::fs::write(
            format!("{}/aegis-timezone.conf", upgrade_override_dir),
            upgrade_content,
        )
        .await
        .context("写入 apt-daily-upgrade.timer override 失败")?;

        let daily_override_dir = "/etc/systemd/system/apt-daily.timer.d";
        tokio::fs::create_dir_all(daily_override_dir)
            .await
            .context("创建 apt-daily.timer.d 目录失败")?;

        let daily_content = AutoUpdateConfigurator::apt_daily_timer_override();
        tokio::fs::write(
            format!("{}/aegis-timezone.conf", daily_override_dir),
            daily_content,
        )
        .await
        .context("写入 apt-daily.timer override 失败")?;

        run_cmd_checked("systemctl", &["daemon-reload"], TIMEOUT_SHORT)
            .await
            .context("systemctl daemon-reload 失败")?;

        Ok(())
    }

    pub async fn reboot_system() -> Result<()> {
        if REBOOT_FLAG.swap(true, Ordering::SeqCst) {
            anyhow::bail!("❌ 重启任务正在执行中，请稍后再试");
        }
        let _guard = FlagGuard(&REBOOT_FLAG);

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
        let config = AutoUpdateConfigurator::generate_config(DistroFamily::Debian, "03:00");
        assert!(config.contains("Allowed-Origins"));
        assert!(config.contains("security"));
        assert!(config.contains("AutoFixInterruptedDpkg"));
        assert!(config.contains("Automatic-Reboot"));
        assert!(config.contains("Automatic-Reboot-Time"));
        assert!(config.contains("03:00"));
        assert!(config.contains("Remove-Unused-Dependencies"));
        assert!(!config.contains("MailOnlyOnError"));
        assert!(!config.contains("Unattended-Upgrade \"1\""));
    }

    #[test]
    fn test_debian_config_custom_reboot_time() {
        let config = AutoUpdateConfigurator::generate_config(DistroFamily::Debian, "05:30");
        assert!(config.contains("Automatic-Reboot-Time \"05:30\";"));
    }

    #[test]
    fn test_rhel_config_content() {
        let config = AutoUpdateConfigurator::generate_config(DistroFamily::Rhel, "03:00");
        assert!(config.contains("upgrade_type"));
        assert!(config.contains("security"));
        assert!(config.contains("download_updates"));
        assert!(config.contains("apply_updates"));
        assert!(config.contains("reboot"));
        assert!(config.contains("when-needed"));
        assert!(config.contains("emit_via"));
        assert!(config.contains("motd"));
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

    #[test]
    fn test_distro_periodic_config_path() {
        assert!(DistroFamily::Debian.periodic_config_path().is_some());
        assert!(DistroFamily::Rhel.periodic_config_path().is_none());
    }

    #[test]
    fn test_distro_needrestart_conf_path() {
        assert!(
            DistroFamily::Debian
                .needrestart_conf_path()
                .contains("needrestart")
        );
        assert!(
            DistroFamily::Rhel
                .needrestart_conf_path()
                .contains("needrestart")
        );
    }

    #[test]
    fn test_supplementary_packages_debian() {
        let pkgs = DistroFamily::Debian.supplementary_packages();
        assert!(pkgs.contains(&"needrestart"));
    }

    #[test]
    fn test_supplementary_packages_rhel() {
        let pkgs = DistroFamily::Rhel.supplementary_packages();
        assert!(pkgs.contains(&"needrestart"));
    }

    #[test]
    fn test_debian_periodic_config() {
        let config = AutoUpdateConfigurator::debian_periodic_config();
        assert!(config.contains("APT::Periodic::Update-Package-Lists \"1\""));
        assert!(config.contains("APT::Periodic::Unattended-Upgrade \"1\""));
        assert!(config.contains("APT::Periodic::AutocleanInterval \"7\""));
    }

    #[test]
    fn test_needrestart_config() {
        let config = AutoUpdateConfigurator::needrestart_config();
        assert!(config.contains("$nrconf{restart} = 'a'"));
    }

    #[test]
    fn test_supplementary_packages_contains_needrestart() {
        assert!(
            DistroFamily::Debian
                .supplementary_packages()
                .contains(&"needrestart")
        );
        assert!(
            DistroFamily::Rhel
                .supplementary_packages()
                .contains(&"needrestart")
        );
    }

    #[test]
    fn test_needrestart_conf_path() {
        assert_eq!(
            DistroFamily::Debian.needrestart_conf_path(),
            "/etc/needrestart/needrestart.conf"
        );
        assert_eq!(
            DistroFamily::Rhel.needrestart_conf_path(),
            "/etc/needrestart/needrestart.conf"
        );
    }

    #[test]
    fn test_apt_daily_timer_override_content() {
        let content = AutoUpdateConfigurator::apt_daily_timer_override();
        assert!(content.contains("OnCalendar=daily"));
        assert!(content.contains("RandomizedDelaySec=4h"));
    }

    #[test]
    fn test_apt_daily_upgrade_timer_override_content() {
        let content = AutoUpdateConfigurator::apt_daily_upgrade_timer_override();
        assert!(content.contains("OnCalendar=daily"));
        assert!(content.contains("RandomizedDelaySec=4h"));
        assert!(content.contains("After=apt-daily.service"));
    }

    #[test]
    fn test_default_reboot_time_is_0500() {
        assert_eq!(Operations::DEFAULT_REBOOT_TIME, "05:00");
    }

    #[test]
    fn test_timeout_short_is_reasonable() {
        assert_eq!(TIMEOUT_SHORT.as_secs(), 10);
    }
}
