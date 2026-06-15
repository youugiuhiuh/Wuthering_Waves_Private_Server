use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, TimeoutStatus};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::prelude::{CallbackQuery, ChatId, Requester, ResponseResult};

pub fn handle_callback(
    bot: Bot,
    mut q: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        loop {
            let user_id = q.from.id.0 as i64;
            if !state.is_authorized(user_id).await {
                bot.answer_callback_query(q.id)
                    .text("🚫 会话已过期，请发送 6 位 TOTP 验证码重新认证")
                    .await?;
                break Ok(());
            }

            let data = match q.data.as_ref() {
                Some(d) => d.clone(),
                None => break Ok(()),
            };
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

            if destruct_flow::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
                == MessageFlowOutcome::Handled
            {
                break Ok(());
            }

            let is_custom_followup = data.starts_with("s_custom_ui:")
                || data.starts_with("s_custom_set:")
                || data == "s_custom_confirm"
                || data == "s_custom_cancel";
            if is_custom_followup
                && state
                    .schedule_timeout_status(chat_id, Duration::from_secs(180))
                    .await
                    == TimeoutStatus::Expired
            {
                state.remove_schedule_input(chat_id).await;
                let new_q = q.clone();
                q = CallbackQuery {
                    data: Some("s_add_custom_menu".to_string()),
                    ..new_q
                };
                bot.answer_callback_query(q.id.clone())
                    .text("⏳ 自定义定时会话已超时，请重新进入。")
                    .show_alert(true)
                    .await?;
                continue;
            }

            if destruct_flow::handle_callback_action(
                &bot,
                &q,
                data.as_str(),
                chat_id,
                msg_id,
                &state,
            )
            .await?
                == MessageFlowOutcome::Handled
            {
                break Ok(());
            }

            let ctx = super::context::CallbackContext {
                bot: bot.clone(),
                q: q.clone(),
                state: state.clone(),
                chat_id,
                msg_id,
                user_id,
                data: data.clone(),
            };

            match super::dispatch(&ctx).await {
                Ok(Some(action)) => match action {
                    super::context::HandlerAction::Done => break Ok(()),
                    super::context::HandlerAction::Redirect(new_data) => {
                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some(new_data),
                            ..new_q
                        };
                        continue;
                    }
                },
                Ok(None) => {} // No handler matched
                Err(e) => {
                    eprintln!("[ERROR] Handler dispatch failed: {:?}", e);
                    let _ = bot
                        .answer_callback_query(q.id.clone())
                        .text("❌ 内部运维业务错误，请查看后台日志")
                        .show_alert(true)
                        .await;
                    break Ok(());
                }
            }

            bot.answer_callback_query(q.id).await?;
            break Ok(());
        }
    })
}
