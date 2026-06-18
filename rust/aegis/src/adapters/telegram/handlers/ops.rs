use super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::schedule::{build_custom_schedule_keyboard, build_custom_schedule_text};
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use aegis::adapters::common::TargetId;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::system::operations::{Operations, REBOOT_FLAG};
use aegis::core::system::scheduler::TaskType;
use aegis::core::system::upgrade::UpgradeManager;
use rust_i18n::t;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "a_reload" => {
            let _ = MaintenanceManager::reload_core().await;
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.reload_success"))
                .await?;
        }
        "a_fw" => {
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                let bot_for_updates = bot_clone.clone();
                let update_task = tokio::spawn(async move {
                    let mut last_text = String::new();
                    while let Some(text) = rx.recv().await {
                        if text == last_text {
                            continue;
                        }
                        last_text = text.clone();
                        let _ = bot_for_updates
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                t!("ops.fw_title", "0" => text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                });

                let tx_clone = tx.clone();
                let res_timeout = tokio::time::timeout(
                    Duration::from_secs(45),
                    MaintenanceManager::harden_firewall(move |text| {
                        let _ = tx_clone.send(text.to_string());
                    }),
                )
                .await;

                match res_timeout {
                    Ok(Ok(_)) => {}
                    Ok(Err(err)) => {
                        let _ = tx.send(t!("ops.fw_fail", "0" => err.to_string()).to_string());
                    }
                    Err(_) => {
                        let _ = tx.send(t!("ops.fw_timeout").to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.fw_start"))
                .await?;
        }
        "a_upgrade" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.upgrade_start"))
                .await?;
            let adapter = ctx.state.adapter.clone();
            let target = TargetId(ctx.chat_id.0.to_string());
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                match UpgradeManager::new() {
                    Ok(manager) => {
                        if let Err(err) = manager.run(adapter.as_ref(), &target).await {
                            let _ = bot_clone
                                .send_message(
                                    chat_id_clone,
                                    t!("ops.upgrade_fail", "0" => err.to_string()),
                                )
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                    }
                    Err(err) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id_clone,
                                t!("ops.upgrade_init_fail", "0" => err.to_string()),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });
        }
        "a_geo" => {
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;

            tokio::spawn(async move {
                let bot_for_cb = bot_clone.clone();
                let progress_cb = move |_: f64, text: &str| {
                    let bot = bot_for_cb.clone();
                    let text = text.to_string();
                    tokio::spawn(async move {
                        let _ = bot
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                t!("ops.geo_title", "0" => text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    });
                };

                match MaintenanceManager::update_geodata(progress_cb).await {
                    Ok(_) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, t!("ops.geo_success"))
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, t!("ops.geo_fail", "0" => e.to_string()))
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.geo_start"))
                .await?;
        }
        "a_tune" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.tune_start"))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                let bot_for_updates = bot_clone.clone();
                let update_task = tokio::spawn(async move {
                    let mut last_text = String::new();
                    while let Some(text) = rx.recv().await {
                        if text == last_text {
                            continue;
                        }
                        last_text = text.clone();
                        let _ = bot_for_updates
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                format!("⚙️ <b>{}</b>\n{}", t!("menu.generic_tune"), text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                });

                let result = MaintenanceManager::tune_vps_generic().await;
                match result {
                    Ok(()) => {
                        let _ = tx.send(t!("ops.tune_success").to_string());
                    }
                    Err(e) => {
                        let _ = tx.send(t!("ops.tune_fail", "0" => e.to_string()).to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });
        }
        "a_sys_update" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.sys_update_start"))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                let bot_for_updates = bot_clone.clone();
                let update_task = tokio::spawn(async move {
                    let mut last_text = String::new();
                    while let Some(text) = rx.recv().await {
                        if text == last_text {
                            continue;
                        }
                        last_text = text.clone();
                        let _ = bot_for_updates
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                format!("⬆️ <b>{}</b>\n{}", t!("menu.sys_cmd"), text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                });

                let tx_clone = tx.clone();
                let result = MaintenanceManager::upgrade_system_packages(move |text| {
                    let _ = tx_clone.send(text.to_string());
                })
                .await;

                match result {
                    Ok(()) => {
                        let _ = tx.send(t!("ops.sys_update_success").to_string());
                    }
                    Err(e) => {
                        let _ =
                            tx.send(t!("ops.sys_update_fail", "0" => e.to_string()).to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });
        }
        "a_bbr3" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.bbr3_start"))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                let bot_for_updates = bot_clone.clone();
                let update_task = tokio::spawn(async move {
                    let mut last_text = String::new();
                    while let Some(text) = rx.recv().await {
                        if text == last_text {
                            continue;
                        }
                        last_text = text.clone();
                        let _ = bot_for_updates
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                t!("ops.bbr3_title", "0" => text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                });

                let tx_clone = tx.clone();
                let res = tokio::time::timeout(
                    Duration::from_secs(300),
                    MaintenanceManager::install_bbr3(move |desc| {
                        let _ = tx_clone.send(desc.to_string());
                    }),
                )
                .await;

                let mut reboot_needed = false;

                match res {
                    Ok(Ok(status)) => {
                        reboot_needed = status.reboot_required;
                        let reboot_text = if status.reboot_required {
                            t!("ops.bbr3_reboot_needed").to_string()
                        } else {
                            String::new()
                        };
                        let _ = tx.send(t!("ops.bbr3_success", "0" => status.kernel_version, "1" => status.congestion_control, "2" => reboot_text).to_string());
                    }
                    Ok(Err(err)) => {
                        let _ = tx.send(t!("ops.bbr3_fail", "0" => err.to_string()).to_string());
                    }
                    Err(_) => {
                        let _ = tx.send(t!("ops.bbr3_timeout").to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;

                if reboot_needed {
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            t!("ops.bbr3_reboot_now"),
                            "a_bbr3_reboot_now",
                        )],
                        vec![InlineKeyboardButton::callback(
                            t!("ops.bbr3_reboot_later"),
                            "a_bbr3_reboot_later",
                        )],
                    ]);
                    let _ = bot_clone
                        .send_message(chat_id_clone, t!("ops.bbr3_reboot_prompt"))
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await;
                }
            });
        }
        "a_sys_maint" => {
            let chat_id_str = ctx.chat_id.0.to_string();
            ctx.state.remove_schedule_input(&chat_id_str).await;
            ctx.state
                .insert_schedule_input(
                    chat_id_str,
                    ScheduleInputState {
                        updated_at: Instant::now(),
                        task_type: TaskType::SecurityUpdate,
                        frequency: ScheduleFrequency::Daily,
                        timezone: "UTC".to_string(),
                        day_of_week: None,
                        hour: None,
                        minute: None,
                        return_to: "m_sys_cmd".to_string(),
                    },
                )
                .await;

            let Some(input_state) = ctx
                .state
                .schedule_input_snapshot(&ctx.chat_id.0.to_string())
                .await
            else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("ops.init_fail"))
                    .await?;
                return Ok(HandlerAction::Done);
            };
            let text = build_custom_schedule_text(&input_state);
            let ret = input_state.return_to.clone();

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(build_custom_schedule_keyboard(&ret))
                .await?;
        }
        "a_sys_reboot" => {
            if REBOOT_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("ops.sys_reboot_busy"))
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("ops.sys_reboot_disabled"),
                "a_sys_reboot_disabled",
            )]]);
            let _ = ctx
                .bot
                .edit_message_reply_markup(ctx.chat_id, ctx.msg_id)
                .reply_markup(keyboard)
                .await;

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.sys_reboot_text"))
                .await?;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = Operations::reboot_system().await;
            });
        }
        "a_bbr3_reboot_now" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.sys_reboot_text"))
                .await?;
            ctx.bot
                .send_message(ctx.chat_id, t!("ops.bbr3_reboot_now_msg"))
                .parse_mode(ParseMode::Html)
                .await?;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = Operations::reboot_system().await;
            });
        }
        "a_bbr3_reboot_later" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.sys_reboot_later"))
                .await?;
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("ops.bbr3_reboot_later_msg"))
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback(t!("menu.back_net_opt"), "m_net_opt"),
                ]]))
                .await?;
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }
    Ok(HandlerAction::Done)
}
