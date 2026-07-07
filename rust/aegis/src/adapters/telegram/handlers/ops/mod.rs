use super::context::{CallbackContext, HandlerAction, HandlerResult};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let _ = ctx;
    Ok(HandlerAction::Done)
}
