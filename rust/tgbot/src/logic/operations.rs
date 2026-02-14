use anyhow::{Context, Result};

use tokio::time::Duration;

use crate::logic::cmd_async::run_cmd_output;

const TIMEOUT_APT: Duration = Duration::from_secs(120);
const TIMEOUT_REBOOT: Duration = Duration::from_secs(15);

pub struct Operations;

impl Operations {
    /// 执行完整的系统维护：更新、升级、清理
    pub async fn perform_maintenance() -> Result<String> {
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
        let (status, _out, stderr) = run_cmd_output("reboot", &[], TIMEOUT_REBOOT)
            .await
            .context("❌ 执行重启命令失败")?;

        if !status.success() {
            anyhow::bail!("❌ 重启命令执行失败: {}", stderr);
        }
        Ok(())
    }

    /// 辅助函数：运行 apt 命令
    async fn run_apt(args: &[&str]) -> Result<()> {
        let (status, _out, stderr) = run_cmd_output("apt-get", args, TIMEOUT_APT)
            .await
            .context(format!("❌ 执行 apt-get 命令 {:?} 失败", args))?;

        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("❌ apt-get 命令 {:?} 执行失败: {}", args, stderr)
        }
    }
}
