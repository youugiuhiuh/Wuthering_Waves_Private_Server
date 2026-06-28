use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::spawn_progress_updater;
use aegis::core::system::maintenance::MaintenanceManager;
use rust_i18n::t;
use std::time::Duration;
use teloxide::prelude::*;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.fw_start"))
        .await?;

    let bot = ctx.bot.clone();
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(bot.clone(), chat_id, msg_id, |t| {
            t!("ops.fw_title", "0" => t).to_string()
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

    Ok(HandlerAction::Done)
}
