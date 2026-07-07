use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::super::schedule::{build_custom_schedule_keyboard, build_custom_schedule_text};
use aegis::app::state::{ScheduleFrequency, ScheduleInputState};
use aegis::core::system::scheduler::TaskType;
use rust_i18n::t;
use std::time::Instant;
use teloxide::prelude::*;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let chat_id_str = ctx.chat_id.0.to_string();
    ctx.state.remove_schedule_input(&chat_id_str).await;
    ctx.state
        .insert_schedule_input(
            chat_id_str,
            ScheduleInputState {
                updated_at: Instant::now(),
                task_type: TaskType::SecurityUpdate,
                frequency: ScheduleFrequency::Daily,
                timezone: "UTC".to_string(),
                day_of_week: None,
                hour: None,
                minute: None,
                return_to: "m_sys_cmd".to_string(),
            },
        )
        .await;

    let Some(input_state) = ctx
        .state
        .schedule_input_snapshot(&ctx.chat_id.0.to_string())
        .await
    else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("ops.init_fail"))
            .await?;
        return Ok(HandlerAction::Done);
    };
    let text = build_custom_schedule_text(&input_state);
    let ret = input_state.return_to.clone();

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(build_custom_schedule_keyboard(&ret))
        .await?;
    Ok(HandlerAction::Done)
}
