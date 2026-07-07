use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::handlers::context::HandlerContext;
use aegis::adapters::common::TargetId;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let hctx = HandlerContext {
        adapter: &*ctx.state.adapter,
        target: TargetId(ctx.chat_id.0.to_string()),
        state: &ctx.state,
        user_id: ctx.user_id,
        data: ctx.data.clone(),
        msg_id: Some(aegis::adapters::common::MessageId(ctx.msg_id.0.to_string())),
    };
    aegis::handlers::ops::handle(&hctx).await.map(HandlerAction::from)
}
