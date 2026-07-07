use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use rust_i18n::t;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_main" => {
            let markup = Markup {
                buttons: vec![vec![InlineButton {
                    text: t!("menu.ops_center").to_string(),
                    data: "m_ops_center".to_string(),
                }]],
            };
            ctx.reply_markup(t!("menu.main_title").to_string(), markup)
                .await?;
            Ok(HandlerAction::Done)
        }
        _ => Ok(HandlerAction::Done),
    }
}
