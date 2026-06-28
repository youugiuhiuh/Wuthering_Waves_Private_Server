use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::spawn_progress_updater;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::system::operations::Operations;
use rust_i18n::t;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    match ctx.data.as_str() {
        "a_bbr3" => handle_prompt(ctx).await,
        "a_bbr3_confirm" => handle_install(ctx).await,
        "a_bbr3_cancel" => handle_cancel(ctx).await,
        "a_bbr3_reboot_now" => handle_reboot_now(ctx).await,
        "a_bbr3_reboot_later" => handle_reboot_later(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_prompt(ctx: &CallbackContext) -> HandlerResult {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            t!("ops.bbr3_confirm_btn"),
            "a_bbr3_confirm",
        )],
        vec![InlineKeyboardButton::callback(
            t!("ops.bbr3_cancel"),
            "a_bbr3_cancel",
        )],
    ]);
    ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("ops.bbr3_confirm_warn"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_install(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.bbr3_start"))
        .await?;

    let bot_clone = ctx.bot.clone();
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(bot_clone.clone(), chat_id, msg_id, |t| {
            t!("ops.bbr3_title", "0" => t).to_string()
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
                let _ = tx.send(
                    t!("ops.bbr3_success",
                        "0" => status.kernel_version,
                        "1" => status.congestion_control,
                        "2" => reboot_text
                    )
                    .to_string(),
                );
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
                .send_message(chat_id, t!("ops.bbr3_reboot_prompt"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await;
        }
    });

    Ok(HandlerAction::Done)
}

async fn handle_cancel(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.bbr3_cancelled"))
        .await?;
    Ok(HandlerAction::Redirect("m_ops_center".to_string()))
}

async fn handle_reboot_now(ctx: &CallbackContext) -> HandlerResult {
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
    Ok(HandlerAction::Done)
}

async fn handle_reboot_later(ctx: &CallbackContext) -> HandlerResult {
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
    Ok(HandlerAction::Done)
}
