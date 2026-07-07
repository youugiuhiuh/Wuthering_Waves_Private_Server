use rust_i18n::t;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use crate::adapters::common::{InlineButton, Markup};
use crate::core::system::SystemMonitor;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "m_main" => handle_main_menu(ctx).await,
        "m_ops_center" => handle_ops_center(ctx).await,
        "m_settings" => handle_settings(ctx).await,
        "m_net_opt" => handle_net_opt(ctx).await,
        "m_security" => handle_security(ctx).await,
        "m_sys_cmd" => handle_sys_cmd(ctx).await,
        "m_mon" => handle_monitor(ctx).await,
        "m_usr" => handle_user_settings(ctx).await,
        "m_danger" => handle_danger_zone(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_main_menu(ctx: &HandlerContext<'_>) -> HandlerResult {
    let report = SystemMonitor::get_status_report()
        .await
        .unwrap_or_else(|e| format!("获取状态失败: {}", e));
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("menu.ops_center").to_string(),
                data: "m_ops_center".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(format!("{}\n\n{}", t!("menu.main_title"), report), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_ops_center(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![
                InlineButton {
                    text: t!("ops.reload").to_string(),
                    data: "a_reload".to_string(),
                },
                InlineButton {
                    text: t!("ops.upgrade").to_string(),
                    data: "a_upgrade".to_string(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("ops.maintenance").to_string(),
                    data: "a_sys_maint".to_string(),
                },
                InlineButton {
                    text: t!("ops.reboot").to_string(),
                    data: "a_sys_reboot".to_string(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("ops.firewall").to_string(),
                    data: "a_fw".to_string(),
                },
                InlineButton {
                    text: t!("ops.bbr3").to_string(),
                    data: "a_bbr3".to_string(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("ops.geo").to_string(),
                    data: "a_geo".to_string(),
                },
                InlineButton {
                    text: t!("ops.tune").to_string(),
                    data: "a_tune".to_string(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("menu.one_click_deploy").to_string(),
                    data: "a_one_click".to_string(),
                },
                InlineButton {
                    text: t!("menu.back_main").to_string(),
                    data: "m_main".to_string(),
                },
            ],
        ],
    };
    ctx.edit_markup(t!("menu.ops_center").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_settings(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("menu.net_opt").to_string(),
                data: "m_net_opt".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.xray_mgmt").to_string(),
                data: "m_xray_mgmt".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.singbox_mgmt").to_string(),
                data: "m_singbox_mgmt".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.warp").to_string(),
                data: "m_warp".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.subscription").to_string(),
                data: "m_sub".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.schedule").to_string(),
                data: "m_sched".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.security").to_string(),
                data: "m_security".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_main").to_string(),
                data: "m_main".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("menu.settings").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_net_opt(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("ops.bbr3").to_string(),
                data: "a_bbr3".to_string(),
            }],
            vec![InlineButton {
                text: t!("ops.tune").to_string(),
                data: "a_tune".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("menu.net_opt").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_security(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("menu.sys_cmd").to_string(),
                data: "m_sys_cmd".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.monitor").to_string(),
                data: "m_mon".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.user_settings").to_string(),
                data: "m_usr".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.danger_zone").to_string(),
                data: "m_danger".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_settings").to_string(),
                data: "m_settings".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("menu.security").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sys_cmd(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("ops.reload").to_string(),
                data: "a_reload".to_string(),
            }],
            vec![InlineButton {
                text: t!("ops.upgrade").to_string(),
                data: "a_upgrade".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_security").to_string(),
                data: "m_security".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("menu.sys_cmd").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_monitor(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("menu.monitor_desc").to_string()).await?;
    Ok(HandlerAction::Done)
}

async fn handle_user_settings(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_security").to_string(),
            data: "m_security".to_string(),
        }]],
    };
    ctx.edit_markup(t!("menu.user_settings").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_danger_zone(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("destruct.destroy_btn").to_string(),
                data: "a_destroy_ask".to_string(),
            }],
            vec![InlineButton {
                text: t!("menu.back_security").to_string(),
                data: "m_security".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("menu.danger_zone_desc").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}
