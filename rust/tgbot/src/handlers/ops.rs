use super::context::{CallbackContext, HandlerAction, HandlerResult};
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::logic::UpgradeManager;
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::{MAINTENANCE_FLAG, Operations, REBOOT_FLAG};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let lang = ctx.state.language().await;
    let data = ctx.data.as_str();
    match data {
        "a_reload" => {
            let _ = MaintenanceManager::reload_core().await;
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.reload", locale = &lang))
                .await?;
        }
        "a_fw" => {
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;
            let lang_for_task = lang.clone();

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                let lang_inner = lang_for_task;
                let lang_for_task = lang_inner.clone();

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
                                format!(
                                    "{} {}\n{}",
                                    t!("ops.firewall", locale = &lang_for_task),
                                    t!("ops.hardening_in_progress", locale = &lang_for_task),
                                    text
                                ),
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
                        let _ = tx.send(
                            t!("ops.firewall_failed", locale = &lang_inner)
                                .replace("%error%", &err.to_string()),
                        );
                    }
                    Err(_) => {
                        let _ =
                            tx.send(t!("ops.firewall_timeout", locale = &lang_inner).to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.firewall_starting", locale = &lang))
                .await?;
        }
        "a_upgrade" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.bot_upgrade_starting", locale = &lang))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let lang_inner = lang.clone();
            tokio::spawn(async move {
                match UpgradeManager::new() {
                    Ok(manager) => {
                        if let Err(err) = manager.run(bot_clone.clone(), chat_id_clone).await {
                            let _ = bot_clone
                                .send_message(
                                    chat_id_clone,
                                    t!("ops.bot_upgrade_failed", locale = &lang_inner)
                                        .replace("%error%", &err.to_string()),
                                )
                                .await;
                        }
                    }
                    Err(err) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id_clone,
                                t!("ops.bot_upgrade_cannot_start", locale = &lang_inner)
                                    .replace("%error%", &err.to_string()),
                            )
                            .await;
                    }
                }
            });
        }
        "a_geo" => {
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;
            let lang_arc = Arc::new(lang.clone());
            let lang_for_success = lang_arc.clone();

            tokio::spawn(async move {
                let bot_for_cb = bot_clone.clone();
                let lang_arc = lang_arc.clone();
                let progress_cb = move |_: f64, text: &str| {
                    let bot = bot_for_cb.clone();
                    let text = text.to_string();
                    let lang = lang_arc.clone();
                    tokio::spawn(async move {
                        let _ = bot
                            .edit_message_text(
                                chat_id_clone,
                                msg_id_clone,
                                format!(
                                    "{} {}\n{}",
                                    t!("ops.geo_updating", locale = lang.as_str()),
                                    t!("ops.hardening_in_progress", locale = lang.as_str()),
                                    text
                                ),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    });
                };

                match MaintenanceManager::update_geodata(progress_cb).await {
                    Ok(_) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id_clone,
                                t!("ops.geo_success", locale = lang_for_success.as_str()),
                            )
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id_clone,
                                t!("ops.geo_failed", locale = lang_for_success.as_str())
                                    .replace("%error%", &e.to_string()),
                            )
                            .await;
                    }
                }
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.geo_started", locale = &lang))
                .await?;
        }
        "a_tune" => {
            return Ok(HandlerAction::Redirect("a_bbr3".to_string()));
        }
        "a_bbr3" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.bbr3_starting", locale = &lang))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let msg_id_clone = ctx.msg_id;
            let lang_for_task = lang.clone();

            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                let lang_inner = lang_for_task;
                let lang_for_task = lang_inner.clone();

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
                                format!(
                                    "{} {}\n{}",
                                    t!("ops.bbr3", locale = &lang_for_task),
                                    t!("ops.bbr3_installing", locale = &lang_for_task),
                                    text
                                ),
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

                match res {
                    Ok(Ok(status)) => {
                        let reboot_text = if status.reboot_required {
                            t!("ops.bbr3_reboot_required", locale = &lang_inner).to_string()
                        } else {
                            String::new()
                        };
                        let _ = tx.send(
                            t!("ops.bbr3_done", locale = &lang_inner)
                                .replace("%kernel%", &status.kernel_version)
                                .replace("%cc%", &status.congestion_control)
                                .replace("%reboot%", &reboot_text),
                        );
                    }
                    Ok(Err(err)) => {
                        let _ = tx.send(
                            t!("ops.bbr3_failed", locale = &lang_inner)
                                .replace("%error%", &err.to_string()),
                        );
                    }
                    Err(_) => {
                        let _ = tx.send(t!("ops.bbr3_timeout", locale = &lang_inner).to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });
        }
        "a_sys_maint" => {
            if MAINTENANCE_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("ops.maint_busy", locale = &lang))
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("ops.maint_configuring", locale = &lang),
                "a_sys_maint_disabled",
            )]]);
            let _ = ctx
                .bot
                .edit_message_reply_markup(ctx.chat_id, ctx.msg_id)
                .reply_markup(keyboard)
                .await;

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.maint_start", locale = &lang))
                .await?;
            let bot_c = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            let lang_inner = lang.clone();
            tokio::spawn(async move {
                match Operations::perform_maintenance().await {
                    Ok(log) => {
                        let log_tail = if log.len() > 4000 {
                            format!("... (Truncated)\n{}", &log[log.len() - 3000..])
                        } else {
                            log
                        };
                        let _ = bot_c
                            .send_message(
                                chat_id_clone,
                                t!("ops.maint_done", locale = &lang_inner)
                                    .replace("%log%", &log_tail),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_c
                            .send_message(
                                chat_id_clone,
                                t!("ops.maint_failed", locale = &lang_inner)
                                    .replace("%error%", &e.to_string()),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });
        }
        "a_sys_reboot" => {
            if REBOOT_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("ops.reboot_busy", locale = &lang))
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("ops.reboot_disabled", locale = &lang),
                "a_sys_reboot_disabled",
            )]]);
            let _ = ctx
                .bot
                .edit_message_reply_markup(ctx.chat_id, ctx.msg_id)
                .reply_markup(keyboard)
                .await;

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.reboot_restarting", locale = &lang))
                .await?;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = Operations::reboot_system().await;
            });
        }
        "a_bbr3_reboot_now" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.reboot_restarting", locale = &lang))
                .await?;
            ctx.bot
                .send_message(ctx.chat_id, t!("ops.reboot_now", locale = &lang))
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
                .text(t!("ops.reboot_later", locale = &lang))
                .await?;
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("ops.reboot_later_msg", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback(t!("ops.net_opt", locale = &lang), "m_net_opt"),
                ]]))
                .await?;
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }
    Ok(HandlerAction::Done)
}
