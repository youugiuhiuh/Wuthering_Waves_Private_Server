use super::context::{HandlerAction, HandlerContext, HandlerResult};

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    let _ = ctx;
    Ok(HandlerAction::Done)
}
