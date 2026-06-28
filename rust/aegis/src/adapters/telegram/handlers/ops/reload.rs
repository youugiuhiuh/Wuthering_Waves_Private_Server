use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::system::maintenance::MaintenanceManager;
use rust_i18n::t;
use teloxide::prelude::*;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let _ = MaintenanceManager::reload_core().await;
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.reload_success"))
        .await?;
    Ok(HandlerAction::Done)
}
