use crate::logic::maintenance::MaintenanceManager;
use crate::logic::operations::Operations;
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
            TaskType::SystemMaintenance => "系统维护+重启 (Maintenance + Reboot)",
            TaskType::GeoUpdate => "GeoData 更新 (Update GeoData)",
            TaskType::Reboot => "系统重启 (Reboot)",
            TaskType::ReloadCore => "重载核心 (Reload Core)",
        }
    }

    pub async fn execute(&self, bot: &Bot, chat_id: ChatId) -> Result<()> {
        match self {
            TaskType::SystemMaintenance => {
                let _ = bot
                    .send_message(
                        chat_id,
                        "🔧 [定时任务] 开始执行系统维护，完成后将自动重启...",
                    )
                    .await;
                let result = Operations::perform_maintenance().await;
                match result {
                    Ok(log_text) => {
                        let log_tail = if log_text.len() > 4000 {
                            format!("... (Truncated)\n{}", &log_text[log_text.len() - 3000..])
                        } else {
                            log_text
                        };
                        bot.send_message(
                            chat_id,
                            format!(
                                "✅ [定时任务] 系统维护完成，3 秒后自动重启。\n\n<pre>{}</pre>",
                                log_tail
                            ),
                        )
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .await?;
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        Operations::reboot_system().await?;
                        Ok(())
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("❌ [定时任务] 系统维护失败: {}", e))
                            .await?;
                        Err(e)
                    }
                }
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
                Operations::reboot_system().await?;
                Ok(())
            }
            TaskType::ReloadCore => {
                let _ = bot.send_message(chat_id, "🔄 重载核心服务...").await;
                MaintenanceManager::reload_core().await?;
                Ok(())
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScheduledTask {
    pub task_type: TaskType,
    pub cron_expression: String,
    #[serde(default = "ScheduledTask::default_timezone")]
    pub timezone: String,
    pub enabled: bool,
}

impl ScheduledTask {
    fn default_timezone() -> String {
        "UTC".to_string()
    }

    pub fn new(task_type: TaskType, cron_expression: &str) -> Self {
        Self::new_with_timezone(task_type, cron_expression, "UTC")
    }

    pub fn new_with_timezone(task_type: TaskType, cron_expression: &str, timezone: &str) -> Self {
        Self {
            task_type,
            cron_expression: cron_expression.to_string(),
            timezone: timezone.to_string(),
            enabled: true,
        }
    }
}
