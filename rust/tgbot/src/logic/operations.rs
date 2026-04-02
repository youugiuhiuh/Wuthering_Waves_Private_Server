use anyhow::{Context, Result};

use once_cell::sync::Lazy;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Duration;

use crate::logic::cmd_async::{run_cmd_checked, run_cmd_output};

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
