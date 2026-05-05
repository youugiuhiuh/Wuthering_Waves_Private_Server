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
                        report_result(
                            bot,
                            chat_id,
                            "系统维护",
                            "✅ [定时任务] 系统维护完成",
                            Err(e),
                        )
                        .await
                    }
                }
            }
            TaskType::GeoUpdate => {
                log::info!("执行 GeoData 更新任务...");
                let _ = bot
                    .send_message(chat_id, "⏳ [定时任务] 开始更新 GeoData...")
                    .await;

                let result = MaintenanceManager::update_geodata(|_pct, msg| {
                    log::info!("[GeoData] {}", msg);
                })
                .await;

                report_result(
                    bot,
                    chat_id,
                    "GeoData 更新",
                    "✅ [定时任务] GeoData 更新完成。",
                    result.map(|_| ()),
                )
                .await
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

async fn report_result(
    bot: &Bot,
    chat_id: ChatId,
    task_name: &str,
    success_msg: &str,
    result: Result<()>,
) -> Result<()> {
    match result {
        Ok(()) => {
            bot.send_message(chat_id, success_msg).await?;
            Ok(())
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ [定时任务] {} 失败: {}", task_name, e))
                .await?;
            Err(e)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_type_display_names() {
        assert_eq!(
            TaskType::SystemMaintenance.get_display_name(),
            "系统维护+重启 (Maintenance + Reboot)"
        );
        assert_eq!(
            TaskType::GeoUpdate.get_display_name(),
            "GeoData 更新 (Update GeoData)"
        );
        assert_eq!(TaskType::Reboot.get_display_name(), "系统重启 (Reboot)");
        assert_eq!(
            TaskType::ReloadCore.get_display_name(),
            "重载核心 (Reload Core)"
        );
    }

    #[test]
    fn test_scheduled_task_new() {
        let task = ScheduledTask::new(TaskType::Reboot, "0 3 * * *");
        assert_eq!(task.cron_expression, "0 3 * * *");
        assert_eq!(task.timezone, "UTC");
        assert!(task.enabled);
        assert_eq!(task.task_type, TaskType::Reboot);
    }

    #[test]
    fn test_scheduled_task_new_with_timezone() {
        let task =
            ScheduledTask::new_with_timezone(TaskType::GeoUpdate, "0 6 * * *", "Asia/Shanghai");
        assert_eq!(task.cron_expression, "0 6 * * *");
        assert_eq!(task.timezone, "Asia/Shanghai");
        assert!(task.enabled);
        assert_eq!(task.task_type, TaskType::GeoUpdate);
    }

    #[test]
    fn test_task_type_serialization() {
        let json = r#""SystemMaintenance""#;
        let task_type: TaskType = serde_json::from_str(json).unwrap();
        assert_eq!(task_type, TaskType::SystemMaintenance);

        let task = ScheduledTask::new(TaskType::ReloadCore, "*/5 * * * *");
        let serialized = serde_json::to_string(&task).unwrap();
        assert!(serialized.contains("ReloadCore"));
        assert!(serialized.contains("*/5 * * * *"));
    }

    #[test]
    fn test_scheduled_task_default_timezone() {
        let json = r#"{"task_type":"GeoUpdate","cron_expression":"0 0 * * *","enabled":true}"#;
        let task: ScheduledTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.timezone, "UTC");
    }

    #[test]
    fn test_scheduled_task_deserialization() {
        let json = r#"{
            "task_type": "Reboot",
            "cron_expression": "0 4 * * *",
            "timezone": "America/New_York",
            "enabled": false
        }"#;
        let task: ScheduledTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.task_type, TaskType::Reboot);
        assert_eq!(task.cron_expression, "0 4 * * *");
        assert_eq!(task.timezone, "America/New_York");
        assert!(!task.enabled);
    }
}
