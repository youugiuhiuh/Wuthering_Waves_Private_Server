use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::xray::config::ConfigManager;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_xray_mgmt" => handle_xray_menu(ctx).await,
        "m_routing" => handle_routing_menu(ctx).await,
        "m_del_cfg" => handle_delete_menu(ctx).await,
        "m_pq_mgmt" => handle_pq_menu(ctx).await,
        "a_inst_base" => handle_install_base(ctx).await,
        data if data.starts_with("u_") => handle_user_list(ctx).await,
        data if data.starts_with("cfg_") => handle_config_detail(ctx).await,
        data if data.starts_with("m_pq_") => handle_pq_action(ctx).await,
        data if data.starts_with("routing_toggle:") => handle_routing_toggle(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_xray_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("xray.routing").to_string(),
                data: "m_routing".to_string(),
            }],
            vec![InlineButton {
                text: t!("xray.delete_cfg").to_string(),
                data: "m_del_cfg".to_string(),
            }],
            vec![InlineButton {
                text: t!("xray.pq_mgmt").to_string(),
                data: "m_pq_mgmt".to_string(),
            }],
            vec![InlineButton {
                text: t!("xray.install_base").to_string(),
                data: "a_inst_base".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("xray.mgmt_title").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_routing_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let rules = ConfigManager::get_warp_routing_rules()
        .await
        .unwrap_or_default();
    let (rules_list, _mode) = rules;
    let text = if rules_list.is_empty() {
        t!("xray.no_routing_rules").to_string()
    } else {
        format!("{}\n\n{}", t!("xray.routing_rules"), rules_list.join("\n"))
    };
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_xray").to_string(),
            data: "m_xray_mgmt".to_string(),
        }]],
    };
    ctx.edit_markup(text, markup).await?;
    Ok(HandlerAction::Done)
}

async fn handle_delete_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let files = ConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    let text = if files.is_empty() {
        t!("xray.no_configs").to_string()
    } else {
        let mut lines = vec![t!("xray.select_del").to_string()];
        for (i, f) in files.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, f));
        }
        lines.join("\n")
    };
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_xray").to_string(),
            data: "m_xray_mgmt".to_string(),
        }]],
    };
    ctx.edit_markup(text, markup).await?;
    Ok(HandlerAction::Done)
}

async fn handle_pq_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let ready = MaintenanceManager::is_reality_base_ready().await;
    let status = if ready {
        t!("xray.pq_ready")
    } else {
        t!("xray.pq_not_ready")
    };
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("xray.pq_gen").to_string(),
                data: "m_pq_gen".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_xray").to_string(),
                data: "m_xray_mgmt".to_string(),
            }],
        ],
    };
    ctx.edit_markup(format!("{}\n\n{}", t!("xray.pq_mgmt"), status), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_install_base(ctx: &HandlerContext<'_>) -> HandlerResult {
    use crate::core::xray::installer::RealityInstallerInternal;
    ctx.edit(t!("xray.installing_base").to_string()).await?;
    match RealityInstallerInternal::install_minimal_environment().await {
        Ok(_) => {
            ctx.reply(t!("xray.install_ok").to_string()).await?;
        }
        Err(e) => {
            ctx.reply(t!("xray.install_fail", "0" => e.to_string()).to_string())
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_user_list(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Done)
}

async fn handle_config_detail(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Done)
}

async fn handle_pq_action(ctx: &HandlerContext<'_>) -> HandlerResult {
    if ctx.data.as_str() == "m_pq_gen" {
        match ConfigManager::generate_reality_pq_keys().await {
            Ok(_) => {
                ctx.edit(t!("xray.pq_gen_ok").to_string()).await?;
            }
            Err(e) => {
                ctx.edit(t!("xray.pq_gen_fail", "0" => e.to_string()).to_string())
                    .await?;
            }
        }
    }
    Ok(HandlerAction::Done)
}

async fn handle_routing_toggle(ctx: &HandlerContext<'_>) -> HandlerResult {
    let _val = ctx.data.trim_start_matches("routing_toggle:").to_string();
    Ok(HandlerAction::Redirect("m_routing".to_string()))
}
