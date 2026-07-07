use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::core::singbox::SingBoxConfigManager;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_singbox_mgmt" => handle_singbox_menu(ctx).await,
        "sb_install" => handle_install(ctx).await,
        data if data.starts_with("sb_") => handle_sb_action(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_singbox_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let files = SingBoxConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    let status = if files.is_empty() {
        t!("singbox.no_configs").to_string()
    } else {
        format!(
            "{} {}\n{}",
            t!("singbox.config_count"),
            files.len(),
            files.join("\n")
        )
    };
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("singbox.install").to_string(),
                data: "sb_install".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(
        format!("{}\n\n{}", t!("singbox.mgmt_title"), status),
        markup,
    )
    .await?;
    Ok(HandlerAction::Done)
}

async fn handle_install(ctx: &HandlerContext<'_>) -> HandlerResult {
    use crate::core::singbox::SingBoxInstaller;
    ctx.edit(t!("singbox.installing").to_string()).await?;
    match SingBoxInstaller::install().await {
        Ok(_) => {
            ctx.reply(t!("singbox.install_ok").to_string()).await?;
        }
        Err(e) => {
            ctx.reply(t!("singbox.install_fail", "0" => e.to_string()).to_string())
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_sb_action(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Done)
}
