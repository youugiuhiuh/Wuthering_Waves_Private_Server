use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::adapters::common::TargetId;
use aegis::core::system::upgrade::UpgradeManager;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
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

    Ok(HandlerAction::Done)
}
