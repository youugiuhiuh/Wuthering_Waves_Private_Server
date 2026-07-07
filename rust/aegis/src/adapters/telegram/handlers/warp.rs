use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::handlers::context::HandlerContext;
use aegis::adapters::common::{MessageId, TargetId};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let hctx = HandlerContext {
        adapter: &*ctx.state.adapter,
        target: TargetId(ctx.chat_id.0.to_string()),
        state: &ctx.state,
        user_id: ctx.user_id,
        data: ctx.data.clone(),
        msg_id: Some(MessageId(ctx.msg_id.0.to_string())),
    };
    aegis::handlers::warp::handle(&hctx).await.map(HandlerAction::from)
}
