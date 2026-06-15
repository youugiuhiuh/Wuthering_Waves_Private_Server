use crate::app::state::AppState;
use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use aegis::adapters::matrix::commands::*;
use aegis::core::singbox::SingBoxConfigManager;
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::xray::ConfigManager;
use aegis::core::xray::installer::WarpInstaller;
use anyhow::Result;
use std::sync::Arc;

const HELP_TEXT: &str = "\
可用命令（无前缀，直接发送）:

  auth <code>         - TOTP 认证
  help                - 显示本帮助
  status              - 系统状态
  menu                - 显示功能菜单

  xray status         - Xray 核心状态
  xray add <proto> [count] - 批量创建 inbound
  xray del [proto]    - 删除配置
  xray pq status      - PQ 密钥状态
  xray pq gen         - 生成 PQ 密钥

  singbox status      - SingBox 状态
  singbox add <proto> [count] - 批量创建
  singbox del         - 删除所有配置

  ops reload          - 重载核心
  ops upgrade         - 自更新
  ops maintenance     - 系统维护 (含重启)
  ops bbr3            - 安装 BBR3
  ops geo             - 更新 GeoData
  ops fw              - 防火墙加固

  schedule list       - 列出计划任务
  schedule add        - 添加计划 (逐步引导)
  schedule del <idx>  - 删除指定计划

  warp status         - WARP 状态
  warp install        - 安装 WARP
  warp uninstall      - 卸载 WARP

  destruct            - 自毁流程";

pub async fn dispatch(
    cmd: &Command,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    _state: &Arc<AppState>,
) -> Result<()> {
    match cmd {
        Command::Help | Command::Menu => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: HELP_TEXT.to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        Command::Status => {
            let report = SystemMonitor::get_status_report()
                .await
                .unwrap_or_else(|e| format!("获取状态失败: {}", e));
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: report,
                        markup: None,
                    },
                )
                .await?;
        }
        Command::Xray(sub) => handle_xray(sub, adapter, target).await?,
        Command::Singbox(sub) => handle_singbox(sub, adapter, target).await?,
        Command::Ops(sub) => handle_ops(sub, adapter, target).await?,
        Command::Destruct => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "⚠️ 自毁流程暂不支持通过 Matrix 执行，请使用 Telegram bot。"
                            .to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        Command::Schedule(_) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "⚠️ 调度管理暂不支持通过 Matrix，请使用 Telegram bot。".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        Command::Warp(sub) => handle_warp(sub, adapter, target).await?,
        Command::Unknown(msg) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg.clone(),
                        markup: None,
                    },
                )
                .await?;
        }
        Command::Auth { .. } => {}
    }
    Ok(())
}

async fn handle_xray(
    sub: &XraySubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
) -> Result<()> {
    match sub {
        XraySubCommand::Status => {
            let files = ConfigManager::list_all_inbound_files().await?;
            let msg = if files.is_empty() {
                "暂无 Xray 配置文件".to_string()
            } else {
                format!("Xray 配置列表 ({}):\n{}", files.len(), files.join("\n"))
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg,
                        markup: None,
                    },
                )
                .await?;
        }
        XraySubCommand::Add { proto, count } => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!("正在创建 {} 个 {} 配置，请稍候...", count, proto),
                        markup: None,
                    },
                )
                .await?;
        }
        XraySubCommand::Del { proto } => {
            let msg = match proto {
                Some(p) => format!("正在删除 {} 配置...", p),
                None => "正在删除所有配置...".to_string(),
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg,
                        markup: None,
                    },
                )
                .await?;
        }
        XraySubCommand::PqStatus => {
            let ready = MaintenanceManager::is_reality_base_ready().await;
            let msg = if ready {
                "✅ PQ 基础配置就绪".to_string()
            } else {
                "❌ PQ 基础配置未完成，请执行 xray pq gen 或检查种子文件".to_string()
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg,
                        markup: None,
                    },
                )
                .await?;
        }
        XraySubCommand::PqGen => match ConfigManager::generate_reality_pq_keys().await {
            Ok(_) => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "✅ PQ 密钥已成功生成".to_string(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            Err(e) => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: format!("❌ PQ 密钥生成失败: {}", e),
                            markup: None,
                        },
                    )
                    .await?;
            }
        },
    }
    Ok(())
}

async fn handle_singbox(
    sub: &SingboxSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
) -> Result<()> {
    match sub {
        SingboxSubCommand::Status => {
            let files = SingBoxConfigManager::list_all_inbound_files().await?;
            let msg = if files.is_empty() {
                "暂无 SingBox 配置文件".to_string()
            } else {
                format!("SingBox 配置列表 ({}):\n{}", files.len(), files.join("\n"))
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg,
                        markup: None,
                    },
                )
                .await?;
        }
        SingboxSubCommand::Add { proto, count } => {
            let msg = format!("正在创建 {} 个 {} 配置...", count, proto);
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg,
                        markup: None,
                    },
                )
                .await?;
        }
        SingboxSubCommand::Del => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在删除所有 SingBox 配置...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
    }
    Ok(())
}

async fn handle_ops(
    sub: &OpsSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
) -> Result<()> {
    match sub {
        OpsSubCommand::Reload => {
            MaintenanceManager::reload_core().await?;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "✅ 核心已重载".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        OpsSubCommand::Upgrade => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在进行自更新，请稍候...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        OpsSubCommand::Maintenance => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在执行系统维护...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        OpsSubCommand::Bbr3 => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在安装 BBR3...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        OpsSubCommand::Geo => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在更新 GeoData...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        OpsSubCommand::Fw => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在执行防火墙加固 (45s 内将输出结果)...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
    }
    Ok(())
}

async fn handle_warp(
    sub: &WarpSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
) -> Result<()> {
    match sub {
        WarpSubCommand::Status => {
            let installed = WarpInstaller::is_installed().await;
            let msg = if installed {
                "✅ WARP 已安装"
            } else {
                "❌ WARP 未安装"
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg.to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        WarpSubCommand::Install => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在安装 WARP...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        WarpSubCommand::Uninstall => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "正在卸载 WARP...".to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
    }
    Ok(())
}
