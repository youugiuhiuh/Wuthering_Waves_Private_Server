use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

pub mod task_types;
pub use task_types::{ScheduledTask, TaskType};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SchedulerState {
    pub tasks: Vec<ScheduledTask>,
}

impl SchedulerState {
    pub fn new() -> Self {
        Self {
            tasks: vec![ScheduledTask::new(TaskType::GeoUpdate, "0 4 * * 0")],
        }
    }

    pub fn default() -> Self {
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

        let mut summary = String::new();
        summary.push_str("⏰ **定时任务列表**:\n\n");

        for (i, task) in self.tasks.iter().enumerate() {
            let status = if task.enabled { "✅" } else { "⏸️" };
            summary.push_str(&format!(
                "{}. {} {}\n   Cron: `{}`\n\n",
                i + 1,
                status,
                task.task_type.get_display_name(),
                task.cron_expression
            ));
        }

        summary
    }
}

pub struct SchedulerManager {
    pub scheduler: Arc<Mutex<Option<JobScheduler>>>,
    pub state: Arc<Mutex<SchedulerState>>,
    pub state_path: String,
}

impl SchedulerManager {
    pub async fn new(bot: Bot, chat_id: ChatId, state_path: String) -> Result<Self> {
        let state = SchedulerState::load_from_file(&state_path)
            .unwrap_or_else(|_| SchedulerState::default());

        if !Path::new(&state_path).exists() {
            let _ = state.save_to_file(&state_path);
        }

        let sched = JobScheduler::new().await?;
        let scheduler = Arc::new(Mutex::new(Some(sched)));
        let state = Arc::new(Mutex::new(state.clone()));

        let manager = Self {
            scheduler,
            state,
            state_path,
        };
        // Pass chat_id.0 (i64) to start_all_tasks
        let _ = manager.start_all_tasks(bot, chat_id.0).await;

        Ok(manager)
    }

    pub async fn start_all_tasks(&self, bot: Bot, chat_id_raw: i64) -> Result<()> {
        let chat_id = ChatId(chat_id_raw);
        let state = self.state.lock().await;
        let tasks = state.tasks.clone();
        drop(state);

        let mut scheduler_guard = self.scheduler.lock().await;

        if let Some(mut sched) = scheduler_guard.take() {
            let _ = sched.shutdown().await;
        }

        // Fixed: removed 'mut' because sched is consumed by valid usage or we don't need mutable access if we just add jobs via method that takes &self (JobScheduler::add takes &self in some versions, or mut self in others.
        // Wait, JobScheduler::add usually takes &self (async).
        // Let's assume &self. If 'mut' was unused, it means it's not needed.
        let sched = JobScheduler::new().await?;

        for task in tasks.iter() {
            if task.enabled {
                let cron_expr = if task.cron_expression.split_whitespace().count() == 5 {
                    format!("0 {}", task.cron_expression)
                } else {
                    task.cron_expression.clone()
                };

                let bot_clone = bot.clone();
                let task_type = task.task_type.clone();

                let job = Job::new_async(cron_expr.as_str(), move |_uuid, _l| {
                    let bot = bot_clone.clone();
                    let task_type = task_type.clone();

                    Box::pin(async move {
                        log::info!("执行定时任务: {:?}", task_type);
                        match task_type.execute(&bot, chat_id).await {
                            Ok(_) => {}
                            Err(e) => {
                                log::error!("任务执行失败: {}", e);
                            }
                        }
                    })
                });

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
        *scheduler_guard = Some(sched);

        Ok(())
    }

    pub async fn add_new_task(
        &self,
        bot: Bot,
        chat_id_raw: i64,
        task: ScheduledTask,
    ) -> Result<String> {
        let validator = SchedulerValidator::new();
        if let Err(validation_error) = validator.validate_cron_expression(&task.cron_expression) {
            return Ok(format!("❌ {}", validation_error));
        }

        let mut state_guard = self.state.lock().await;
        state_guard.add_task(task.clone());
        if let Err(e) = state_guard.save_to_file(&self.state_path) {
            log::error!("保存任务状态失败: {}", e);
        }
        drop(state_guard);

        self.restart_scheduler(bot, ChatId(chat_id_raw)).await?;

        Ok(format!(
            "✅ 新任务已添加: {} ({})",
            task.task_type.get_display_name(),
            task.cron_expression
        ))
    }

    pub async fn remove_task_at(&self, bot: Bot, chat_id_raw: i64, index: usize) -> Result<String> {
        let mut state_guard = self.state.lock().await;
        let result = state_guard.remove_task(index);
        match result {
            Ok(_) => {
                let _ = state_guard.save_to_file(&self.state_path);
                drop(state_guard);
                self.restart_scheduler(bot, ChatId(chat_id_raw)).await?;
                Ok("✅ 任务已删除".to_string())
            }
            Err(e) => Ok(format!("❌ 删除任务失败: {}", e)),
        }
    }

    async fn restart_scheduler(&self, bot: Bot, chat_id: ChatId) -> Result<()> {
        self.start_all_tasks(bot, chat_id.0).await
    }

    pub async fn get_summary(&self) -> String {
        let state_guard = self.state.lock().await;
        state_guard.get_tasks_summary()
    }
}

pub struct SchedulerValidator;

impl SchedulerValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_cron_expression(&self, cron_expr: &str) -> Result<(), String> {
        let fields: Vec<&str> = cron_expr.split_whitespace().collect();
        if fields.len() != 5 && fields.len() != 6 {
            return Err(format!(
                "Invalid cron expression fields count: {}. Expected 5 or 6.",
                fields.len()
            ));
        }
        Ok(())
    }
}

pub static SCHEDULER: Lazy<Arc<Mutex<Option<SchedulerManager>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

pub async fn start_scheduler(bot: Bot, chat_id: ChatId) -> Result<()> {
    log::info!("⏰ 开始初始化调度器...");
    let state_path = "/etc/wwps/tgbot/scheduler_state.json".to_string();

    let manager = SchedulerManager::new(bot, chat_id, state_path).await?;
    let mut manager_guard = SCHEDULER.lock().await;
    *manager_guard = Some(manager);

    log::info!("✅ 调度器初始化完成");
    Ok(())
}
