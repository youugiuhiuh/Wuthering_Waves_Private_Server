use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::app::state::{SubSetupState, SubSetupStep};

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_sub" => handle_sub_menu(ctx).await,
        "sub_setup" => handle_sub_setup(ctx).await,
        "sub_status" => handle_sub_status(ctx).await,
        "sub_del" => handle_sub_del(ctx).await,
        data if data.starts_with("sub_") => handle_sub_action(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_sub_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("sub.setup").to_string(),
                data: "sub_setup".to_string(),
            }],
            vec![InlineButton {
                text: t!("sub.status").to_string(),
                data: "sub_status".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("sub.mgmt_title").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sub_setup(ctx: &HandlerContext<'_>) -> HandlerResult {
    let target_str = ctx.target.0.clone();
    ctx.state
        .insert_sub_setup(
            target_str.clone(),
            SubSetupState {
                step: SubSetupStep::ChooseDomain,
                has_domain: false,
                domain: String::new(),
                port: 0,
                rate_limit: 0,
                tls_mode: 0,
            },
        )
        .await;
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("sub.has_domain").to_string(),
                data: "sub_has_domain".to_string(),
            }],
            vec![InlineButton {
                text: t!("sub.no_domain").to_string(),
                data: "sub_no_domain".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.cancel").to_string(),
                data: "m_sub".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("sub.choose_domain").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sub_status(ctx: &HandlerContext<'_>) -> HandlerResult {
    let text = t!("sub.disabled").to_string();
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_sub").to_string(),
            data: "m_sub".to_string(),
        }]],
    };
    ctx.edit_markup(text, markup).await?;
    Ok(HandlerAction::Done)
}

async fn handle_sub_del(ctx: &HandlerContext<'_>) -> HandlerResult {
    // TODO: full implementation with confirmation
    ctx.reply(t!("sub.deleted").to_string()).await?;
    Ok(HandlerAction::Done)
}

async fn handle_sub_action(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Done)
}

/// Handle text input from subscription wizard (called by message handler)
pub async fn handle_text_input(
    ctx: &HandlerContext<'_>,
    _text: &str,
) -> Result<bool, anyhow::Error> {
    let target_str = ctx.target.0.clone();
    let Some(state) = ctx.state.sub_setup_status(&target_str).await else {
        return Ok(false);
    };
    match state.step {
        SubSetupStep::ChooseDomain => Ok(false),
        SubSetupStep::EnterDomain => {
            ctx.state.remove_sub_setup(&target_str).await;
            // ...full wizard
            Ok(true)
        }
        _ => Ok(false),
    }
}
