use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_sched" => handle_sched_menu(ctx).await,
        data if data.starts_with("s_") => handle_sched_action(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_sched_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("schedule.list").to_string(),
                data: "s_list".to_string(),
            }],
            vec![InlineButton {
                text: t!("schedule.add").to_string(),
                data: "s_add".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("schedule.mgmt_title").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sched_action(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Done)
}
