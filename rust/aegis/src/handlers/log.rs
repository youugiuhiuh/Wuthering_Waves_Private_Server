use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::core::system::log_audit::LogAudit;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_log" => handle_log_menu(ctx).await,
        data if data.starts_with("l_") => handle_log_action(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_log_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("log.xray").to_string(),
                data: "l_xray".to_string(),
            }],
            vec![InlineButton {
                text: t!("log.system").to_string(),
                data: "l_sys".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_main").to_string(),
                data: "m_main".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("log.select").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_log_action(ctx: &HandlerContext<'_>) -> HandlerResult {
    let log_type = ctx.data.trim_start_matches("l_");
    let log_text = match log_type {
        "xray" => LogAudit::tail_logs(crate::core::system::log_audit::SERVICE_WWPS_CORE, 50).await,
        "sys" => LogAudit::tail_logs("syslog", 50).await,
        _ => Ok(t!("log.no_logs").to_string()),
    };
    let text = log_text.unwrap_or_else(|_| t!("log.no_logs").to_string());
    ctx.reply(text).await?;
    Ok(HandlerAction::Done)
}
