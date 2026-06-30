use crate::adapters::common::{BotAdapter, TargetId};
use anyhow::{Context, Result};
use chrono_tz::Tz;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

pub mod task_types;
pub use task_types::{ScheduledTask, TaskType};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SchedulerState {
    pub tasks: Vec<ScheduledTask>,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerState {
    pub fn new() -> Self {
        Self {
            tasks: vec![ScheduledTask::new(TaskType::GeoUpdate, "0 4 * * 0")],
        }
    }

    pub fn get_default() -> Self {
        Self::new()
    }

    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_file(path: &str) -> Result<Self> {
        if !Path::new(path).exists() {
            return Ok(SchedulerState::default());
        }
        let content = fs::read_to_string(path)?;
        let state: SchedulerState = serde_json::from_str(&content)?;
        Ok(state)
    }

    pub fn add_task(&mut self, task: ScheduledTask) {
        self.tasks.push(task);
    }

    pub fn remove_task(&mut self, index: usize) -> Result<()> {
        if index < self.tasks.len() {
            self.tasks.remove(index);
            Ok(())
        } else {
            Err(anyhow::anyhow!("任务索引超出范围"))
        }
    }

    pub fn get_tasks_summary(&self) -> String {
        if self.tasks.is_empty() {
            return "📝 暂无定时任务".to_string();
        }

        let body: String = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, task)| {
                let status = if task.enabled { "✅" } else { "⏸️" };
                format!(
                    "{}. {} {}\n   Cron: `{}`\n   TZ: `{}`\n\n",
                    i + 1,
                    status,
                    task.task_type.get_display_name(),
                    task.cron_expression,
                    task.timezone
                )
            })
            .collect::<Vec<_>>()
            .join("");

        format!("⏰ **定时任务列表**:\n\n{}", body)
    }
}

pub struct SchedulerManager {
    pub scheduler: Arc<Mutex<Option<JobScheduler>>>,
    pub state: Arc<Mutex<SchedulerState>>,
    pub state_path: String,
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
}

impl SchedulerManager {
    pub async fn new(
        adapter: Arc<dyn BotAdapter>,
        target: TargetId,
        state_path: String,
    ) -> Result<Arc<Self>> {
        let path = state_path.clone();
        let state = tokio::task::spawn_blocking(move || {
            let s =
                SchedulerState::load_from_file(&path).unwrap_or_else(|_| SchedulerState::default());
            if !Path::new(&path).exists() {
                let _ = s.save_to_file(&path);
            }
            s
        })
        .await
        .context("scheduler state load")?;

        let sched = JobScheduler::new().await?;
        let scheduler = Arc::new(Mutex::new(Some(sched)));
        let state = Arc::new(Mutex::new(state.clone()));

        let manager = Arc::new(Self {
            scheduler,
            state,
            state_path,
            adapter,
            target,
        });

        let _ = manager.start_all_tasks().await;

        Ok(manager)
    }

    pub async fn start_all_tasks(&self) -> Result<()> {
        let tasks = {
            let state = self.state.lock().await;
            state.tasks.clone()
        };

        let old_sched = {
            let mut guard = self.scheduler.lock().await;
            guard.take()
        };

        if let Some(mut sched) = old_sched {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sched.shutdown()).await;
        }

        let sched = JobScheduler::new().await?;

        for task in tasks.iter() {
            if task.enabled {
                let cron_expr = normalize_cron_expression(&task.cron_expression);
                let job = build_job(
                    self.adapter.clone(),
                    self.target.clone(),
                    task,
                    cron_expr.as_str(),
                );

                match job {
                    Ok(j) => {
                        if let Err(e) = sched.add(j).await {
                            log::error!("添加任务失败: {:?}", e);
                        }
                    }
                    Err(e) => log::error!("创建任务失败 (Cron: {}): {:?}", cron_expr, e),
                }
            }
        }

        let _ = sched.start().await;

        {
            let mut guard = self.scheduler.lock().await;
            *guard = Some(sched);
        }

        Ok(())
    }

    pub async fn add_new_task(&self, task: ScheduledTask) -> Result<String> {
        let validator = SchedulerValidator::new();
        if let Err(validation_error) = validator.validate_task(&task) {
            return Ok(format!("❌ {}", validation_error));
        }

        let previous_state = {
            let state_guard = self.state.lock().await;
            state_guard.clone()
        };

        {
            let mut state_guard = self.state.lock().await;
            state_guard.add_task(task.clone());
        }

        if let Err(err) = self.start_all_tasks().await {
            let mut state_guard = self.state.lock().await;
            *state_guard = previous_state;
            return Err(err);
        }

        let state_guard = self.state.lock().await;
        state_guard.save_to_file(&self.state_path)?;

        Ok(format!(
            "✅ 新任务已添加: {} ({}, {})",
            task.task_type.get_display_name(),
            task.cron_expression,
            task.timezone
        ))
    }

    pub async fn remove_task_at(&self, index: usize) -> Result<String> {
        let previous_state = {
            let state_guard = self.state.lock().await;
            state_guard.clone()
        };

        {
            let mut state_guard = self.state.lock().await;
            let result = state_guard.remove_task(index);
            match result {
                Ok(_) => {}
                Err(e) => return Ok(format!("❌ 删除任务失败: {}", e)),
            }
        }

        if let Err(err) = self.start_all_tasks().await {
            let mut state_guard = self.state.lock().await;
            *state_guard = previous_state;
            return Err(err);
        }

        let state_guard = self.state.lock().await;
        state_guard.save_to_file(&self.state_path)?;
        Ok("✅ 任务已删除".to_string())
    }

    pub async fn get_summary(&self) -> String {
        let state_guard = self.state.lock().await;
        state_guard.get_tasks_summary()
    }
}

pub struct SchedulerValidator;

impl Default for SchedulerValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_cron_expression(&self, cron_expr: &str) -> Result<String, String> {
        let fields: Vec<&str> = cron_expr.split_whitespace().collect();
        if fields.len() != 5 && fields.len() != 6 {
            return Err(format!(
                "Invalid cron expression fields count: {}. Expected 5 or 6.",
                fields.len()
            ));
        }

        let normalized = normalize_cron_expression(cron_expr);
        Job::new_async(normalized.as_str(), |_uuid, _l| Box::pin(async {}))
            .map(|_| normalized)
            .map_err(|e| format!("无效 cron 表达式: {}", e))
    }

    pub fn validate_timezone(&self, timezone: &str) -> Result<(), String> {
        if timezone.trim().is_empty() {
            return Err("时区不能为空".to_string());
        }
        canonical_timezone_name(timezone)
            .parse::<Tz>()
            .map(|_| ())
            .map_err(|_| format!("无效时区: {}", timezone))
    }

    pub fn validate_task(&self, task: &ScheduledTask) -> Result<(), String> {
        let _normalized_cron = self.validate_cron_expression(&task.cron_expression)?;
        self.validate_timezone(&task.timezone)?;
        Ok(())
    }
}

/// Global scheduler: handlers should clone the `Arc<SchedulerManager>` and immediately
/// drop the outer lock to avoid blocking the entire bot while scheduler restarts.
pub static SCHEDULER: Lazy<Arc<Mutex<Option<Arc<SchedulerManager>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

/// Helper: clone the Arc<SchedulerManager> without holding the SCHEDULER lock.
pub async fn get_manager() -> Option<Arc<SchedulerManager>> {
    let guard = SCHEDULER.lock().await;
    guard.as_ref().cloned()
}

/// Initialise the global scheduler, replacing any existing instance.
///
/// # Errors
///
/// Returns an error if the [`SchedulerManager`] cannot be created (e.g.,
/// I/O errors loading persisted state or spawning background jobs).
pub async fn start_scheduler(adapter: Arc<dyn BotAdapter>, target: TargetId) -> Result<()> {
    log::info!("⏰ 开始初始化调度器...");
    let state_path = "/etc/wwps/aegis/scheduler_state.json".to_string();

    let manager = SchedulerManager::new(adapter, target, state_path).await?;
    let mut manager_guard = SCHEDULER.lock().await;
    *manager_guard = Some(manager);

    log::info!("✅ 调度器初始化完成");
    Ok(())
}

fn normalize_cron_expression(cron_expr: &str) -> String {
    if cron_expr.split_whitespace().count() == 5 {
        format!("0 {}", cron_expr)
    } else {
        cron_expr.to_string()
    }
}

fn build_job(
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
    task: &ScheduledTask,
    cron_expr: &str,
) -> Result<Job> {
    let timezone_name = canonical_timezone_name(&task.timezone);

    match timezone_name.parse::<Tz>() {
        Ok(timezone) => {
            let adapter_clone = adapter.clone();
            let target_clone = target.clone();
            let task_type = task.task_type.clone();
            Job::new_async_tz(cron_expr, timezone, move |_uuid, _l| {
                let adapter = adapter_clone.clone();
                let target = target_clone.clone();
                let task_type = task_type.clone();

                Box::pin(async move {
                    log::info!("执行定时任务: {:?}", task_type);
                    if let Err(e) = task_type.execute(&*adapter, &target).await {
                        log::error!("任务执行失败: {}", e);
                    }
                })
            })
            .map_err(Into::into)
        }
        Err(_) => {
            let adapter_clone = adapter.clone();
            let target_clone = target.clone();
            let task_type = task.task_type.clone();
            Job::new_async(cron_expr, move |_uuid, _l| {
                let adapter = adapter_clone.clone();
                let target = target_clone.clone();
                let task_type = task_type.clone();

                Box::pin(async move {
                    log::info!("执行定时任务: {:?}", task_type);
                    if let Err(e) = task_type.execute(&*adapter, &target).await {
                        log::error!("任务执行失败: {}", e);
                    }
                })
            })
            .map_err(Into::into)
        }
    }
}

fn canonical_timezone_name(timezone: &str) -> String {
    let trimmed = timezone.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("UTC") {
        "Etc/UTC".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_cron_expression_rejects_invalid_expression() {
        let validator = SchedulerValidator::new();
        assert!(validator.validate_cron_expression("* *").is_err());
        assert!(
            validator
                .validate_cron_expression("invalid cron expression")
                .is_err()
        );
    }

    #[test]
    fn validate_task_accepts_valid_task() {
        let validator = SchedulerValidator::new();
        let task = ScheduledTask::new_with_timezone(TaskType::GeoUpdate, "0 0 4 * * *", "Etc/UTC");
        assert!(validator.validate_task(&task).is_ok());
    }

    #[test]
    fn validate_task_rejects_invalid_timezone() {
        let validator = SchedulerValidator::new();
        let task =
            ScheduledTask::new_with_timezone(TaskType::GeoUpdate, "0 4 * * 0", "Mars/Phobos");
        assert!(validator.validate_task(&task).is_err());
    }

    #[tokio::test]
    async fn add_new_task_rejects_invalid_task_without_persisting_state() {
        let tempdir = tempdir().unwrap();
        let state_path = tempdir.path().join("scheduler_state.json");
        use crate::adapters::common::{MessageContent, Platform};
        use async_trait::async_trait;

        struct TestAdapter;
        #[async_trait]
        impl BotAdapter for TestAdapter {
            fn platform(&self) -> Platform {
                Platform::Telegram
            }
            async fn send_message(
                &self,
                _target: &TargetId,
                _content: MessageContent,
            ) -> Result<crate::adapters::common::MessageId> {
                Ok(crate::adapters::common::MessageId("0".to_string()))
            }
            async fn edit_message(
                &self,
                _target: &TargetId,
                _msg_id: &crate::adapters::common::MessageId,
                _content: MessageContent,
            ) -> Result<()> {
                Ok(())
            }
            async fn delete_message(
                &self,
                _target: &TargetId,
                _msg_id: &crate::adapters::common::MessageId,
            ) -> Result<()> {
                Ok(())
            }
        }

        let manager = SchedulerManager {
            scheduler: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(SchedulerState { tasks: Vec::new() })),
            state_path: state_path.to_string_lossy().to_string(),
            adapter: Arc::new(TestAdapter),
            target: TargetId("0".to_string()),
        };

        let result = manager
            .add_new_task(ScheduledTask::new_with_timezone(
                TaskType::GeoUpdate,
                "* *",
                "UTC",
            ))
            .await
            .unwrap();

        assert!(result.starts_with("❌"));
        assert!(!state_path.exists());
        assert!(manager.state.lock().await.tasks.is_empty());
    }
}
