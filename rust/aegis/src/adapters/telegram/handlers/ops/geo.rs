use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::system::maintenance::MaintenanceManager;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.geo_start"))
        .await?;

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

    Ok(HandlerAction::Done)
}
