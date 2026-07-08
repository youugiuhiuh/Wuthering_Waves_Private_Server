use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let _ = event;
    Ok(HandlerAction::Done)
}
