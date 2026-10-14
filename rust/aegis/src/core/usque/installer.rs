//! usque 的 I/O 层：下载、校验、注册、安装、卸载、状态探测。
//!
//! # 设计原则
//!
//! 所有**判定**都抽成纯函数（在 [`super::pin`] / [`super::unit`] /
//! [`super::engine`] / [`super::health`]），本文件只做「读状态 → 调纯函数
//! → 执行副作用」。
//!
//! 原因是路径被硬编码为 `/etc/wwps/...`，单测跑不到真实的 I/O 分支；
//! 把判定与副作用分离，判定就能被穷举测试。
//!
//! # 卸载是幂等的
//!
//! 见 [`InstalledKinds`] —— 只装了一套、或两套都没装，都必须返回成功。

use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs;

use crate::core::paths::{usque as usque_paths, warp as warp_paths};

/// 实际安装了哪些 WARP 实现。
///
/// 「安装」的判据是**数据文件存在**，不是「我以为装了什么」——
/// 卸载必须基于真实文件状态，否则会留下幽灵残留。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstalledKinds {
    pub wireguard: bool,
    pub usque: bool,
}

impl InstalledKinds {
    /// 都没装。
    pub const fn none() -> Self {
        Self {
            wireguard: false,
            usque: false,
        }
    }

    /// 至少装了一套。
    pub const fn any(&self) -> bool {
        self.wireguard || self.usque
    }

    /// 由两个「文件是否存在」的布尔值构造。**纯函数，便于穷举测试。**
    pub const fn detect(wireguard_account_exists: bool, usque_config_exists: bool) -> Self {
        Self {
            wireguard: wireguard_account_exists,
            usque: usque_config_exists,
        }
    }
}

/// 卸载结果——供 bot 文案决定「实际卸了什么」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UninstallOutcome {
    pub removed_wireguard: bool,
    pub removed_usque: bool,
}

impl UninstallOutcome {
    /// 什么都没卸（两套本来就没装）。
    pub const fn nothing_removed(&self) -> bool {
        !self.removed_wireguard && !self.removed_usque
    }

    /// i18n key，4 种组合互不相同。
    pub const fn i18n_key(&self) -> &'static str {
        match (self.removed_wireguard, self.removed_usque) {
            (true, true) => "warp.uninstalled_both",
            (true, false) => "warp.uninstalled_wireguard",
            (false, true) => "warp.uninstalled_usque",
            (false, false) => "warp.uninstalled_none",
        }
    }
}

/// 读取当前实际安装状态。
pub async fn detect_installed() -> InstalledKinds {
    let wg = fs::try_exists(warp_paths::ACCOUNT_FILE)
        .await
        .unwrap_or(false);
    let uq = fs::try_exists(usque_paths::CONFIG_FILE)
        .await
        .unwrap_or(false);
    InstalledKinds::detect(wg, uq)
}

/// 卸载所有已安装的 WARP 实现（WireGuard + usque）。
///
/// # 幂等
///
/// 两套都没装时返回 `Ok(UninstallOutcome::default())`，**不是错误**。
/// 卸载一个本来就不存在的东西是成功，不是失败——否则管理员会在
/// 「点了一次卸载没反应，再点一次报错」的循环里困惑。
///
/// # 不因单步失败而中断
///
/// 每一步（stop / disable / 删文件）的失败都被记录并继续。
/// 整体只要「尽力清干净」；最后清理 Xray 路由配置并重载核心。
pub async fn uninstall_all() -> Result<UninstallOutcome> {
    let installed = detect_installed().await;
    let mut outcome = UninstallOutcome::default();

    if installed.wireguard {
        remove_wireguard_files().await;
        outcome.removed_wireguard = true;
    }

    if installed.usque {
        remove_usque().await;
        outcome.removed_usque = true;
    }

    // 无论卸了哪一套，路由配置都可能引用 WARP 出口——清掉它，
    // 让核心回到无 WARP 状态。这一步在什么都没卸时也执行：
    // 路由文件可能早于账号文件被手工删除而残留。
    let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;

    crate::core::system::maintenance::MaintenanceManager::reload_core().await?;

    Ok(outcome)
}

async fn remove_wireguard_files() {
    // 账号文件：失败只记日志，不阻断（文件可能已被手工删除）。
    if let Err(e) = fs::remove_file(warp_paths::ACCOUNT_FILE).await {
        log::warn!("删除 WARP 账号文件失败（忽略）: {e}");
    }
}

async fn remove_usque() {
    // 1. 先停服务，再摘 unit——顺序反了会让 systemd 去拉一个已不存在的 unit。
    for action in ["stop", "disable"] {
        if let Err(e) = run_systemctl(&[action, usque_paths::SERVICE_NAME]).await {
            // 服务本来就没运行 / 没 enable 时也会非零退出，属正常。
            log::debug!(
                "systemctl {action} {} 失败（可能本就未启用）: {e}",
                usque_paths::SERVICE_NAME
            );
        }
    }

    // 2. 删 unit 文件并让 systemd 重新读取。
    if Path::new(usque_paths::SERVICE_PATH).exists()
        && let Err(e) = fs::remove_file(usque_paths::SERVICE_PATH).await
    {
        log::warn!("删除 usque unit 文件失败（忽略）: {e}");
    }
    let _ = run_systemctl(&["daemon-reload"]).await;

    // 3. 删账号配置与二进制。
    //    账号文件含已注册的 Cloudflare access_token，必须清掉。
    for path in [usque_paths::CONFIG_FILE, usque_paths::BIN] {
        if let Err(e) = fs::remove_file(path).await {
            log::debug!("删除 {path} 失败（可能本就不存在）: {e}");
        }
    }

    // 4. 目录本身（若已空）。
    let _ = fs::remove_dir(usque_paths::DIR).await;
}

async fn run_systemctl(args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .context("执行 systemctl 失败")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "systemctl {:?} 退出码 {:?}: {}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 穷举 4 种组合。
    #[test]
    fn test_detect_exhaustive() {
        assert_eq!(
            InstalledKinds::detect(true, true),
            InstalledKinds {
                wireguard: true,
                usque: true
            }
        );
        assert_eq!(
            InstalledKinds::detect(true, false),
            InstalledKinds {
                wireguard: true,
                usque: false
            }
        );
        assert_eq!(
            InstalledKinds::detect(false, true),
            InstalledKinds {
                wireguard: false,
                usque: true
            }
        );
        assert_eq!(InstalledKinds::detect(false, false), InstalledKinds::none());
    }

    #[test]
    fn test_installed_kinds_any() {
        assert!(!InstalledKinds::none().any());
        assert!(InstalledKinds::detect(true, false).any());
        assert!(InstalledKinds::detect(false, true).any());
        assert!(InstalledKinds::detect(true, true).any());
    }

    /// 4 种卸载结果的 i18n key 必须互不相同——否则 UI 会显示错文案。
    #[test]
    fn test_outcome_i18n_keys_are_distinct() {
        let combos = [
            UninstallOutcome {
                removed_wireguard: true,
                removed_usque: true,
            },
            UninstallOutcome {
                removed_wireguard: true,
                removed_usque: false,
            },
            UninstallOutcome {
                removed_wireguard: false,
                removed_usque: true,
            },
            UninstallOutcome::default(),
        ];
        let keys: Vec<&str> = combos.iter().map(|o| o.i18n_key()).collect();
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 4, "4 种组合的 i18n key 必须互不相同: {keys:?}");
        for k in keys {
            assert!(k.starts_with("warp."), "key 应位于 warp 命名空间: {k}");
        }
    }

    #[test]
    fn test_outcome_nothing_removed() {
        assert!(UninstallOutcome::default().nothing_removed());
        let only_wg = UninstallOutcome {
            removed_wireguard: true,
            removed_usque: false,
        };
        assert!(!only_wg.nothing_removed());
        let only_uq = UninstallOutcome {
            removed_wireguard: false,
            removed_usque: true,
        };
        assert!(!only_uq.nothing_removed());
    }

    /// 「什么都没卸」必须是**成功**语义，不是错误。
    ///
    /// 幂等：卸载一个本就不存在的东西算成功。
    #[test]
    fn test_nothing_removed_is_not_an_error_state() {
        let o = UninstallOutcome::default();
        assert!(o.nothing_removed());
        assert_eq!(o.i18n_key(), "warp.uninstalled_none");
        // 该 key 用于「未检测到 WARP 实现」的**成功**提示
    }
}
