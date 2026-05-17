use super::context::{CallbackContext, HandlerAction, HandlerResult};
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::logic::UpgradeManager;
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::{MAINTENANCE_FLAG, Operations, REBOOT_FLAG};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "a_reload" => {
            let _ = MaintenanceManager::reload_core().await;
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("✅ 已重启核心")
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
                                format!("🛡️ <b>防火墙安全加固</b>\n{}", text),
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
                        let _ = tx.send(format!("❌ 失败: {}", err));
                    }
                    Err(_) => {
                        let _ = tx
                            .send("❌ 失败: 操作超时 (45s)，请检查系统 nftables 状态".to_string());
                    }
                }

                drop(tx);
                let _ = update_task.await;
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⚙️ 正在启动防火墙扫描与加固...")
                .await?;
        }
        "a_upgrade" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⚙️ 正在启动 Bot 自更新...")
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                match UpgradeManager::new() {
                    Ok(manager) => {
                        if let Err(err) = manager.run(bot_clone.clone(), chat_id_clone).await {
                            let _ = bot_clone
                                .send_message(chat_id_clone, format!("❌ 自更新失败: {}", err))
                                .await;
                        }
                    }
                    Err(err) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, format!("❌ 无法启动自更新: {}", err))
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
                                format!("🌍 <b>GeoData 更新中</b>\n{}", text),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    });
                };

                match MaintenanceManager::update_geodata(progress_cb).await {
                    Ok(_) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, "✅ GeoData 更新成功")
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(chat_id_clone, format!("❌ GeoData 更新失败: {}", e))
                            .await;
                    }
                }
            });

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("🌍 GeoData 已启动更新 (后台执行)")
                .await?;
        }
        "a_tune" => {
            return Ok(HandlerAction::Redirect("a_bbr3".to_string()));
        }
        "a_bbr3" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("🚀 正在启动 BBR3 安装向导...")
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
                                format!("🚀 <b>BBR3 安装中</b>\n{}", text),
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
                            "\n\n🔄 <b>需要重启系统才能启用 BBR3</b>\n\n点击「立即重启」按钮，或稍后手动执行 reboot 命令。"
                        } else {
                            ""
                        };
                        let _ = tx.send(format!(
                            "✅ <b>BBR3 安装完成</b>\n\n内核: {}\n拥塞控制: {}{}",
                            status.kernel_version, status.congestion_control, reboot_text
                        ));
                    }
                    Ok(Err(err)) => {
                        let _ = tx.send(format!("❌ BBR3 安装失败: {}", err));
                    }
                    Err(_) => {
                        let _ = tx.send("❌ BBR3 安装超时 (5分钟)，请稍后重试".to_string());
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
                    .text("❌ 配置任务正在执行中，请稍后再试")
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "⚙️ 配置中... (请等待)",
                "a_sys_maint_disabled",
            )]]);
            let _ = ctx
                .bot
                .edit_message_reply_markup(ctx.chat_id, ctx.msg_id)
                .reply_markup(keyboard)
                .await;

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⚙️ 正在配置自动安全更新...")
                .await?;
            let bot_c = ctx.bot.clone();
            let chat_id = ctx.chat_id;
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
                                chat_id,
                                format!(
                                    "✅ <b>自动安全更新配置完成</b>\n\n<pre>{}</pre>",
                                    log_tail
                                ),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    Err(e) => {
                        let _ = bot_c
                            .send_message(chat_id, format!("❌ <b>维护失败</b>\n\n原因: {}", e))
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
                    .text("❌ 重启任务正在执行中，请稍后再试")
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "⚠️ 重启中... (请等待)",
                "a_sys_reboot_disabled",
            )]]);
            let _ = ctx
                .bot
                .edit_message_reply_markup(ctx.chat_id, ctx.msg_id)
                .reply_markup(keyboard)
                .await;

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⚠️ 系统将于 3 秒后重启...")
                .await?;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = Operations::reboot_system().await;
            });
        }
        "a_bbr3_reboot_now" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text("⚠️ 系统将于 3 秒后重启...")
                .await?;
            ctx.bot
                .send_message(
                    ctx.chat_id,
                    "⚠️ <b>系统将于 3 秒后重启</b>\nBBR3 新内核将在重启后生效。",
                )
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
                .text("✅ 已选择稍后重启")
                .await?;
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "✅ <b>已记录为稍后重启</b>\n\nBBR3 已安装完成，待你手动重启系统后切换到新内核生效。",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("⬅️ 返回网络优化", "m_net_opt"),
                ]]))
                .await?;
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }
    Ok(HandlerAction::Done)
}
