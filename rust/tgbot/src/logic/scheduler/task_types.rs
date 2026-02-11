use crate::logic::maintenance::MaintenanceManager;
use crate::logic::operations::Operations;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use teloxide::Bot;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TaskType {
    SystemMaintenance,
    Reboot,
    ReloadCore,
    GeoUpdate,
}

impl TaskType {
    pub fn get_display_name(&self) -> &str {
        match self {
            TaskType::SystemMaintenance => "系统全面维护 (Update & Clean)",
            TaskType::Reboot => "系统安全重启",
            TaskType::ReloadCore => "重启核心服务 (wwps-core/wwps-box)",
            TaskType::GeoUpdate => "更新 GeoData (geosite/geoip)",
        }
    }

    pub async fn execute(&self, bot: &Bot, chat_id: ChatId) -> Result<()> {
        match self {
            TaskType::SystemMaintenance => {
                let _ = bot
                    .send_message(chat_id, "⏰ <b>定时任务启动</b>: 系统全面维护...")
                    .parse_mode(ParseMode::Html)
                    .await;
                match Operations::perform_maintenance().await {
                    Ok(log) => {
                        let log_tail = if log.len() > 1000 {
                            format!("... (Truncated)\n{}", &log[log.len() - 1000..])
                        } else {
                            log
                        };
                        let _ = bot
                            .send_message(
                                chat_id,
                                format!("✅ <b>定时维护完成</b>\n<pre>{}</pre>", log_tail),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot
                            .send_message(chat_id, format!("❌ <b>定时维护失败</b>: {}", e))
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            }
            TaskType::Reboot => {
                let _ = bot
                    .send_message(chat_id, "⏰ <b>定时任务启动</b>: 系统将在 5 秒后重启...")
                    .parse_mode(ParseMode::Html)
                    .await;
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let _ = Operations::reboot_system().await;
            }
            TaskType::ReloadCore => match MaintenanceManager::reload_core().await {
                Ok(_) => {
                    let _ = bot
                        .send_message(chat_id, "⏰ <b>定时任务</b>: 核心服务已重启")
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                Err(e) => {
                    let _ = bot
                        .send_message(chat_id, format!("❌ <b>定时重启核心失败</b>: {}", e))
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            },
            TaskType::GeoUpdate => {
                let msg = bot
                    .send_message(chat_id, "⏰ <b>定时任务启动</b>: 正在准备更新 GeoData...")
                    .parse_mode(ParseMode::Html)
                    .await?;
                let bot_clone = bot.clone();
                let chat_id_clone = chat_id;
                let msg_id = msg.id;

                let progress_cb = move |_: f64, text: &str| {
                    let bot = bot_clone.clone();
                    let text = text.to_string();
                    tokio::spawn(async move {
                        let _ = bot
                            .edit_message_text(
                                chat_id_clone,
                                msg_id,
                                format!("⏰ <b>GeoData 更新中</b>\n{}", text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    });
                };

                match MaintenanceManager::update_geodata(progress_cb).await {
                    Ok(_) => {
                        let _ = bot
                            .edit_message_text(
                                chat_id,
                                msg_id,
                                "✅ <b>定时任务</b>: GeoData 已更新并重载核心",
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot
                            .edit_message_text(
                                chat_id,
                                msg_id,
                                format!("❌ <b>GeoData 更新失败</b>: {}", e),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            }
        }
        Ok(())
    }
}
