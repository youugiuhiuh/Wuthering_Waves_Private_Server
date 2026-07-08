use aegis::adapters::common::TargetId;
use aegis::shared::handlers::message::MessageAction;
use rust_i18n::t;
use std::sync::Arc;
use teloxide::Bot;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Message, Requester, ResponseResult};
use teloxide::types::ParseMode;

use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::AppState;

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let Some(from) = msg.from.as_ref() else {
        bot.send_message(chat_id, t!("auth.invalid_user"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    };
    let user_id = from.id.0 as i64;

    if !state.is_admin_user(user_id) {
        return Ok(());
    }

    let target = TargetId(chat_id.0.to_string());
    let text = msg.text();
    let has_file = msg.document().is_some() || msg.photo().is_some();

    match aegis::shared::handlers::message::handle_message(
        &*state.adapter,
        &target,
        text,
        has_file,
        &*state as &dyn aegis::shared::handlers::message::MessageState,
    )
    .await
    {
        Ok(MessageAction::Handled) => return Ok(()),
        Ok(MessageAction::NeedsDestruct) => {}
        Err(e) => {
            log::error!("Shared message handler error: {:?}", e);
        }
    }

    if destruct_flow::handle_message_flow(&bot, &msg, user_id, &state).await?
        == MessageFlowOutcome::Handled
    {
        return Ok(());
    }

    if let Some(text) = msg.text() {
        let code = text.trim();
        if crate::looks_like_totp_code(code) && !state.is_authorized(user_id).await {
            let _ = crate::process_auth_code(&state, &target, user_id, code).await;
            return Ok(());
        }
    }

    Ok(())
}
