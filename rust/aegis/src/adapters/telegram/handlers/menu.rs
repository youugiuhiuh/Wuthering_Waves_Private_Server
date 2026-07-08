use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::bootstrap::{BotSettings, DEFAULT_SESSION_TIMEOUT_SECS};
use aegis::adapters::common::{MessageId, TargetId};
use aegis::shared::types::CallbackEvent;

fn convert_action(action: aegis::shared::types::HandlerAction) -> HandlerAction {
    match action {
        aegis::shared::types::HandlerAction::Done => HandlerAction::Done,
        aegis::shared::types::HandlerAction::Redirect(data) => HandlerAction::Redirect(data),
    }
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();

    if data.starts_with("set_timeout:") {
        let secs: u64 = data
            .strip_prefix("set_timeout:")
            .unwrap_or("0")
            .parse()
            .unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS);
        ctx.state.set_session_timeout_secs(secs).await;
        let settings = BotSettings {
            session_timeout_secs: secs,
        };
        if let Err(e) = settings.save() {
            log::error!("保存会话设置失败: {}", e);
        }
    }

    let event = CallbackEvent {
        adapter: ctx.state.adapter.clone(),
        target: TargetId(ctx.chat_id.0.to_string()),
        user_id: ctx.user_id.to_string(),
        msg_id: MessageId(ctx.msg_id.0.to_string()),
        data: ctx.data.clone(),
        callback_id: ctx.q.id.clone(),
        session_timeout_secs: ctx.state.session_timeout_secs().await,
    };

    let shared_result = aegis::shared::handlers::menu::handle(&event).await?;
    Ok(convert_action(shared_result))
}
