use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::system::operations::{Operations, REBOOT_FLAG};
use rust_i18n::t;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
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
    Ok(HandlerAction::Done)
}
