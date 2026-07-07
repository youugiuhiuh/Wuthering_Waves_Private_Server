use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::core::xray::installer::WarpInstaller;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_warp" => handle_warp_menu(ctx).await,
        "a_warp_install" => handle_warp_install(ctx).await,
        "a_warp_uninstall" => handle_warp_uninstall(ctx).await,
        "a_warp_status" => handle_warp_status(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_warp_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let installed = WarpInstaller::is_installed().await;
    let status = if installed {
        t!("warp.installed")
    } else {
        t!("warp.not_installed")
    };
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("warp.status").to_string(),
                data: "a_warp_status".to_string(),
            }],
            vec![if installed {
                InlineButton {
                    text: t!("warp.uninstall").to_string(),
                    data: "a_warp_uninstall".to_string(),
                }
            } else {
                InlineButton {
                    text: t!("warp.install").to_string(),
                    data: "a_warp_install".to_string(),
                }
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(format!("{}\n\n{}", t!("warp.mgmt_title"), status), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_warp_install(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("warp.installing").to_string()).await?;
    match WarpInstaller::install().await {
        Ok(_) => {
            ctx.reply(t!("warp.install_ok").to_string()).await?;
        }
        Err(e) => {
            ctx.reply(t!("warp.install_fail", "0" => e.to_string()).to_string())
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_warp_uninstall(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("warp.uninstalling").to_string()).await?;
    match WarpInstaller::uninstall().await {
        Ok(_) => {
            ctx.reply(t!("warp.uninstall_ok").to_string()).await?;
        }
        Err(e) => {
            ctx.reply(t!("warp.uninstall_fail", "0" => e.to_string()).to_string())
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_warp_status(ctx: &HandlerContext<'_>) -> HandlerResult {
    let installed = WarpInstaller::is_installed().await;
    let status = if installed {
        t!("warp.installed")
    } else {
        t!("warp.not_installed")
    };
    ctx.reply(status.to_string()).await?;
    Ok(HandlerAction::Done)
}
