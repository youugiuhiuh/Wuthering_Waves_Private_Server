use crate::logic::maintenance::MaintenanceManager;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use teloxide::prelude::*;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TaskType {
    SystemMaintenance,
    GeoUpdate,  // Matches main.rs
    Reboot,     // Matches main.rs
    ReloadCore, // Matches main.rs
}

impl TaskType {
    pub fn get_display_name(&self) -> &str {
        match self {
            TaskType::SystemMaintenance => "系统维护 (System Maintenance)",
            TaskType::GeoUpdate => "GeoData 更新 (Update GeoData)",
            TaskType::Reboot => "系统重启 (Reboot)",
            TaskType::ReloadCore => "重载核心 (Reload Core)",
        }
    }

    pub async fn execute(&self, bot: &Bot, chat_id: ChatId) -> Result<()> {
        match self {
            TaskType::SystemMaintenance => {
                let _ = bot.send_message(chat_id, "🔧 执行系统维护...").await;
                // TODO: Implement actual maintenance logic if needed
                Ok(())
            }
            TaskType::GeoUpdate => {
                log::info!("执行 GeoData 更新任务...");
                let _ = bot
                    .send_message(chat_id, "⏳ [定时任务] 开始更新 GeoData...")
                    .await;

                // Use a simple callback for logging
                let result = MaintenanceManager::update_geodata(|_pct, msg| {
                    log::info!("[GeoData] {}", msg);
                })
                .await;

                match result {
                    Ok(_) => {
                        bot.send_message(chat_id, "✅ [定时任务] GeoData 更新完成。")
                            .await?;
                        Ok(())
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ [定时任务] GeoData 更新失败: {}", e))
                            .await?;
                        Err(e)
                    }
                }
            }
            TaskType::Reboot => {
                let _ = bot
                    .send_message(chat_id, "⚠️ 系统即将重启 (定时任务)...")
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                std::process::Command::new("reboot").spawn()?;
                Ok(())
            }
            TaskType::ReloadCore => {
                let _ = bot.send_message(chat_id, "🔄 重载核心服务...").await;
                std::process::Command::new("systemctl")
                    .arg("restart")
                    .arg("wwps-core")
                    .output()?;
                Ok(())
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScheduledTask {
    pub task_type: TaskType,
    pub cron_expression: String,
    pub enabled: bool,
}

impl ScheduledTask {
    pub fn new(task_type: TaskType, cron_expression: &str) -> Self {
        Self {
            task_type,
            cron_expression: cron_expression.to_string(),
            enabled: true,
        }
    }
}
