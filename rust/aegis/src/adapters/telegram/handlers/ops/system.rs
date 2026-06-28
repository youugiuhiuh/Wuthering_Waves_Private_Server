use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::spawn_progress_updater;
use aegis::core::system::maintenance::MaintenanceManager;
use rust_i18n::t;
use teloxide::prelude::*;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    match ctx.data.as_str() {
        "a_tune" => handle_tune(ctx).await,
        "a_sys_update" => handle_sys_update(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_tune(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.tune_start"))
        .await?;

    let bot = ctx.bot.clone();
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(bot.clone(), chat_id, msg_id, |t| {
            format!("⚙️ <b>{}</b>\n{}", t!("menu.generic_tune"), t)
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

    Ok(HandlerAction::Done)
}

async fn handle_sys_update(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.sys_update_start"))
        .await?;

    let bot = ctx.bot.clone();
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(bot.clone(), chat_id, msg_id, |t| {
            format!("⬆️ <b>{}</b>\n{}", t!("menu.sys_cmd"), t)
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
                let _ = tx.send(t!("ops.sys_update_fail", "0" => e.to_string()).to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}
