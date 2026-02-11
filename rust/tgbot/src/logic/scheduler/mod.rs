use crate::logic::scheduler::task_types::TaskType;
use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use teloxide::Bot;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

pub mod task_types;

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
        Self { tasks: vec![] }
    }

    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
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
            Err(anyhow::anyhow!("索引越界"))
        }
    }

    pub fn get_all_tasks_summary(&self) -> String {
        if self.tasks.is_empty() {
            return "📝 暂无定时任务".to_string();
        }
        let mut summary = String::new();
        for (i, task) in self.tasks.iter().enumerate() {
            let status = if task.enabled { "✅" } else { "⏸️" };
            summary.push_str(&format!(
                "{}. {} {}\n   Cron: {}\n",
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
    pub scheduler: Arc<Mutex<JobScheduler>>,
    pub state: Arc<Mutex<SchedulerState>>,
    pub state_path: String,
}

impl SchedulerManager {
    pub async fn new(
        bot: Bot,
        admin_id: i64,
        state_path: String,
    ) -> Result<Self, JobSchedulerError> {
        let state_data = SchedulerState::load_from_file(&state_path).unwrap_or_default();

        let manager = Self {
            scheduler: Arc::new(Mutex::new(JobScheduler::new().await?)),
            state: Arc::new(Mutex::new(state_data)),
            state_path,
        };
        let _ = manager.start_all_tasks(bot, admin_id).await;

        Ok(manager)
    }

    pub async fn start_all_tasks(&self, bot: Bot, admin_id: i64) -> Result<(), JobSchedulerError> {
        let state = self.state.lock().await;
        let tasks = state.tasks.clone();
        drop(state);

        let mut scheduler = self.scheduler.lock().await;
        scheduler.shutdown().await?;
        *scheduler = JobScheduler::new().await?;
        scheduler.start().await?;

        for task in tasks {
            if task.enabled {
                let cron_expr = if task.cron_expression.split_whitespace().count() == 5 {
                    format!("0 {}", task.cron_expression) // Add seconds if missing
                } else {
                    task.cron_expression.clone()
                };

                let bot_clone = bot.clone();
                let task_type_clone = task.task_type.clone();
                let admin_id_clone = admin_id;

                let job =
                    Job::new_async_tz(cron_expr.as_str(), chrono::Local, move |_uuid, _l| {
                        let bot = bot_clone.clone();
                        let task_type = task_type_clone.clone();
                        let admin = admin_id_clone;

                        Box::pin(async move {
                            let _ = task_type.execute(&bot, ChatId(admin)).await;
                        })
                    })?;

                scheduler.add(job).await?;
            }
        }

        Ok(())
    }

    pub async fn add_new_task(&self, bot: Bot, admin_id: i64, task: ScheduledTask) -> Result<()> {
        let mut state = self.state.lock().await;
        state.add_task(task);
        let _ = state.save_to_file(&self.state_path);
        drop(state);

        let _ = self.start_all_tasks(bot, admin_id).await;
        Ok(())
    }

    pub async fn remove_task_at(&self, bot: Bot, admin_id: i64, index: usize) -> Result<()> {
        let mut state = self.state.lock().await;
        state.remove_task(index)?;
        let _ = state.save_to_file(&self.state_path);
        drop(state);

        let _ = self.start_all_tasks(bot, admin_id).await;
        Ok(())
    }

    pub async fn get_summary(&self) -> String {
        let state = self.state.lock().await;
        state.get_all_tasks_summary()
    }
}

// Global Singleton for easy access from main handlers
pub static SCHEDULER: Lazy<Arc<Mutex<Option<SchedulerManager>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

pub async fn init_scheduler(bot: Bot, admin_id: i64, state_path: String) -> Result<()> {
    let manager = SchedulerManager::new(bot, admin_id, state_path).await?;
    let mut global_sched = SCHEDULER.lock().await;
    *global_sched = Some(manager);
    Ok(())
}
