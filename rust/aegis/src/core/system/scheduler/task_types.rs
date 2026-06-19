use crate::adapters::common::{BotAdapter, MessageContent, TargetId};
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::system::operations::Operations;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TaskType {
    GeoUpdate,
    Reboot,
    ReloadCore,
    SecurityUpdate,
    #[serde(other)]
    Unknown,
}

impl TaskType {
    pub fn get_display_name(&self) -> &str {
        match self {
            TaskType::GeoUpdate => "GeoData 更新 (Update GeoData)",
            TaskType::Reboot => "系统重启 (Reboot)",
            TaskType::ReloadCore => "重载核心 (Reload Core)",
            TaskType::SecurityUpdate => "安全更新 (Security Update)",
            TaskType::Unknown => "未知任务 (已弃用)",
        }
    }

    pub async fn execute(&self, adapter: &dyn BotAdapter, target: &TargetId) -> Result<()> {
        match self {
            TaskType::GeoUpdate => {
                log::info!("执行 GeoData 更新任务...");
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "⏳ [定时任务] 开始更新 GeoData...".to_string(),
                            markup: None,
                        },
                    )
                    .await;

                let result = MaintenanceManager::update_geodata(|_pct, msg| {
                    log::info!("[GeoData] {}", msg);
                })
                .await;

                report_result(
                    adapter,
                    target,
                    "GeoData 更新",
                    "✅ [定时任务] GeoData 更新完成。",
                    result.map(|_| ()),
                )
                .await
            }
            TaskType::Reboot => {
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "⚠️ 系统即将重启 (定时任务)...".to_string(),
                            markup: None,
                        },
                    )
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                Operations::reboot_system().await?;
                Ok(())
            }
            TaskType::ReloadCore => {
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "🔄 重载核心服务...".to_string(),
                            markup: None,
                        },
                    )
                    .await;
                MaintenanceManager::reload_core().await?;
                Ok(())
            }
            TaskType::SecurityUpdate => {
                log::info!("执行安全更新定时任务...");
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "⏳ [定时任务] 开始执行安全更新...".to_string(),
                            markup: None,
                        },
                    )
                    .await;

                let result = Operations::perform_security_update_task().await;

                report_result(
                    adapter,
                    target,
                    "安全更新",
                    "✅ [定时任务] 安全更新执行完成。",
                    result,
                )
                .await
            }
            TaskType::Unknown => {
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "⚠️ 此任务类型已弃用，自动安全更新已由系统定时器管理。"
                                .to_string(),
                            markup: None,
                        },
                    )
                    .await;
                Ok(())
            }
        }
    }
}

async fn report_result(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    task_name: &str,
    success_msg: &str,
    result: Result<()>,
) -> Result<()> {
    match result {
        Ok(()) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: success_msg.to_string(),
                        markup: None,
                    },
                )
                .await?;
            Ok(())
        }
        Err(e) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!("❌ [定时任务] {} 失败: {:#}", task_name, e),
                        markup: None,
                    },
                )
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
    fn test_unknown_variant_deserialization() {
        let json = r#""SystemMaintenance""#;
        let task_type: TaskType = serde_json::from_str(json).unwrap();
        assert_eq!(task_type, TaskType::Unknown);

        let json2 = r#""Unknown""#;
        let task_type2: TaskType = serde_json::from_str(json2).unwrap();
        assert_eq!(task_type2, TaskType::Unknown);
    }

    #[test]
    fn test_unknown_variant_serialization() {
        let serialized = serde_json::to_string(&TaskType::Unknown).unwrap();
        assert_eq!(serialized, r#""Unknown""#);
    }

    #[test]
    fn test_unknown_display_name() {
        assert_eq!(TaskType::Unknown.get_display_name(), "未知任务 (已弃用)");
    }

    #[test]
    fn test_task_type_display_names() {
        assert_eq!(TaskType::Unknown.get_display_name(), "未知任务 (已弃用)");
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
    fn test_security_update_display_name() {
        assert_eq!(
            TaskType::SecurityUpdate.get_display_name(),
            "安全更新 (Security Update)"
        );
    }

    #[test]
    fn test_security_update_serialization() {
        let json = r#""SecurityUpdate""#;
        let task_type: TaskType = serde_json::from_str(json).unwrap();
        assert_eq!(task_type, TaskType::SecurityUpdate);
    }

    #[test]
    fn test_task_type_serialization() {
        let json = r#""SystemMaintenance""#;
        let task_type: TaskType = serde_json::from_str(json).unwrap();
        assert_eq!(task_type, TaskType::Unknown);

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
