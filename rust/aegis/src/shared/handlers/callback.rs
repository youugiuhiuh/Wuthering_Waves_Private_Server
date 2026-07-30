use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};

#[allow(dead_code)]
pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let _ = event;
    Ok(HandlerAction::Done)
}
