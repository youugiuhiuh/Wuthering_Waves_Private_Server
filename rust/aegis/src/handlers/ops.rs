use std::time::Duration;

use rust_i18n::t;
use tokio::sync::mpsc;

use super::context::{HandlerAction, HandlerContext, HandlerResult};
use super::progress::spawn_progress_updater;
use crate::adapters::common::{InlineButton, Markup, MessageContent};
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::system::operations::Operations;

pub async fn handle(ctx: &HandlerContext<'_>) -> HandlerResult {
    match ctx.data.as_str() {
        "a_reload" => handle_reload(ctx).await,
        "a_fw" => handle_firewall(ctx).await,
        "a_upgrade" => handle_upgrade(ctx).await,
        "a_geo" => handle_geo(ctx).await,
        "a_tune" | "a_sys_update" => handle_tune(ctx).await,
        "a_bbr3" => handle_bbr3_prompt(ctx).await,
        "a_bbr3_confirm" => handle_bbr3_install(ctx).await,
        "a_bbr3_cancel" => handle_bbr3_cancel(ctx).await,
        "a_bbr3_reboot_now" => handle_bbr3_reboot_now(ctx).await,
        "a_bbr3_reboot_later" => handle_bbr3_reboot_later(ctx).await,
        "a_sys_maint" => handle_sys_maint(ctx).await,
        "a_sys_reboot" => handle_reboot(ctx).await,
        "a_one_click" => handle_deploy(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_reload(ctx: &HandlerContext<'_>) -> HandlerResult {
    MaintenanceManager::reload_core().await?;
    ctx.reply(t!("ops.reload_success").to_string()).await?;
    Ok(HandlerAction::Done)
}

async fn handle_firewall(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("ops.fw_start").to_string()).await?;
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    let msg_id = ctx.msg_id.clone();
    tokio::spawn(async move {
        if let Some(msg_id) = msg_id {
            let (tx, update_task) =
                spawn_progress_updater(adapter.clone(), target.clone(), msg_id, |t| {
                    t!("ops.fw_title", "0" => t).to_string()
                });
            let tx_clone = tx.clone();
            let res = tokio::time::timeout(
                Duration::from_secs(45),
                MaintenanceManager::harden_firewall(move |text| {
                    let _ = tx_clone.send(text.to_string());
                }),
            )
            .await;
            match res {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    let _ = tx.send(t!("ops.fw_fail", "0" => e.to_string()).to_string());
                }
                Err(_) => {
                    let _ = tx.send(t!("ops.fw_timeout").to_string());
                }
            }
            drop(tx);
            let _ = update_task.await;
        }
    });
    Ok(HandlerAction::Done)
}

async fn handle_upgrade(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("ops.upgrade_start").to_string()).await?;
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    tokio::spawn(async move {
        match crate::core::system::upgrade::UpgradeManager::new() {
            Ok(manager) => {
                if let Err(e) = manager.run(adapter.as_ref(), &target).await {
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: t!("ops.upgrade_fail", "0" => e.to_string()).to_string(),
                                markup: None,
                            },
                        )
                        .await;
                }
            }
            Err(e) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.upgrade_fail", "0" => e.to_string()).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    });
    Ok(HandlerAction::Done)
}

async fn handle_geo(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("ops.geo_start").to_string()).await?;
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    tokio::spawn(async move {
        let progress_adapter = adapter.clone();
        let progress_target = target.clone();
        let progress_cb = move |_: f64, text: &str| {
            let pa = progress_adapter.clone();
            let pt = progress_target.clone();
            let t = text.to_string();
            tokio::spawn(async move {
                let _ = pa
                    .send_message(
                        &pt,
                        MessageContent {
                            text: t!("ops.geo_title", "0" => t).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            });
        };
        match MaintenanceManager::update_geodata(progress_cb).await {
            Ok(_) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.geo_success").to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.geo_fail", "0" => e.to_string()).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    });
    Ok(HandlerAction::Done)
}

async fn handle_tune(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.edit(t!("ops.tune_start").to_string()).await?;
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    tokio::spawn(async move {
        match MaintenanceManager::tune_vps_generic().await {
            Ok(_) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.tune_success").to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.tune_fail", "0" => e.to_string()).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    });
    Ok(HandlerAction::Done)
}

async fn handle_bbr3_prompt(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("ops.bbr3_confirm_btn").to_string(),
                data: "a_bbr3_confirm".to_string(),
            }],
            vec![InlineButton {
                text: t!("ops.bbr3_cancel").to_string(),
                data: "a_bbr3_cancel".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("ops.bbr3_confirm_warn").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_bbr3_install(ctx: &HandlerContext<'_>) -> HandlerResult {
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    let msg = ctx.reply(t!("ops.bbr3_start").to_string()).await?;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(adapter.clone(), target.clone(), msg, |t| {
            t!("ops.bbr3_title", "0" => t).to_string()
        });

        let tx_clone = tx.clone();
        let res = tokio::time::timeout(
            Duration::from_secs(300),
            MaintenanceManager::install_bbr3(move |desc| {
                let _ = tx_clone.send(desc.to_string());
            }),
        )
        .await;

        let mut reboot_needed = false;

        match res {
            Ok(Ok(status)) => {
                reboot_needed = status.reboot_required;
                let reboot_text = if status.reboot_required {
                    t!("ops.bbr3_reboot_needed").to_string()
                } else {
                    String::new()
                };
                let _ = tx.send(
                    t!("ops.bbr3_success",
                        "0" => status.kernel_version,
                        "1" => status.congestion_control,
                        "2" => reboot_text
                    )
                    .to_string(),
                );
            }
            Ok(Err(err)) => {
                let _ = tx.send(t!("ops.bbr3_fail", "0" => err.to_string()).to_string());
            }
            Err(_) => {
                let _ = tx.send(t!("ops.bbr3_timeout").to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;

        if reboot_needed {
            let keyboard_markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("ops.bbr3_reboot_now").to_string(),
                        data: "a_bbr3_reboot_now".to_string(),
                    }],
                    vec![InlineButton {
                        text: t!("ops.bbr3_reboot_later").to_string(),
                        data: "a_bbr3_reboot_later".to_string(),
                    }],
                ],
            };
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("ops.bbr3_reboot_prompt").to_string(),
                        markup: Some(keyboard_markup),
                    },
                )
                .await;
        }
    });

    Ok(HandlerAction::Done)
}

async fn handle_bbr3_cancel(_ctx: &HandlerContext<'_>) -> HandlerResult {
    Ok(HandlerAction::Redirect("m_ops_center".to_string()))
}

async fn handle_bbr3_reboot_now(ctx: &HandlerContext<'_>) -> HandlerResult {
    ctx.reply(t!("ops.bbr3_reboot_now_msg").to_string()).await?;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = Operations::reboot_system().await;
    });
    Ok(HandlerAction::Done)
}

async fn handle_bbr3_reboot_later(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_net_opt").to_string(),
            data: "m_net_opt".to_string(),
        }]],
    };
    ctx.edit_markup(t!("ops.bbr3_reboot_later_msg").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sys_maint(ctx: &HandlerContext<'_>) -> HandlerResult {
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    let msg = ctx.reply(t!("ops.maint_start").to_string()).await?;

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(adapter.clone(), target.clone(), msg, |t| {
            t!("ops.maint_title", "0" => t).to_string()
        });

        let result =
            Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME).await;

        match result {
            Ok(_) => {
                let _ = tx.send(t!("ops.maint_success").to_string());
            }
            Err(e) => {
                let _ = tx.send(t!("ops.maint_fail", "0" => e.to_string()).to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

async fn handle_reboot(ctx: &HandlerContext<'_>) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("ops.reboot_confirm").to_string(),
                data: "a_reboot_confirm".to_string(),
            }],
            vec![InlineButton {
                text: t!("ops.reboot_cancel").to_string(),
                data: "m_ops_center".to_string(),
            }],
        ],
    };
    ctx.edit_markup(t!("ops.reboot_prompt").to_string(), markup)
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_deploy(ctx: &HandlerContext<'_>) -> HandlerResult {
    let adapter = ctx.state.adapter.clone();
    let target = ctx.target.clone();
    let msg = ctx.reply(t!("ops.deploy_start").to_string()).await?;

    tokio::spawn(async move {
        use crate::core::xray::installer::RealityInstallerInternal;

        let (tx, update_task) = spawn_progress_updater(adapter.clone(), target.clone(), msg, |t| {
            format!("{} {}", t!("menu.one_click_deploy"), t)
        });

        let mut failed = false;
        let mut all_links: Vec<String> = Vec::new();

        send_progress(&tx, 1, 8, t!("ops.deploy_step_tune"));
        if MaintenanceManager::tune_vps_generic().await.is_err() {
            let _ = tx.send(t!("ops.deploy_fail", "0" => t!("ops.deploy_fail_tune")).to_string());
            failed = true;
        }

        let xray_installed = tokio::fs::try_exists(crate::core::paths::xray::BIN)
            .await
            .unwrap_or(false);
        if xray_installed {
            send_progress(
                &tx,
                2,
                8,
                format!("{} - ⏩ skipped", t!("ops.deploy_step_xray_init")),
            );
        } else if !failed {
            send_progress(&tx, 2, 8, t!("ops.deploy_step_xray_init"));
            if let Err(e) = RealityInstallerInternal::install_minimal_environment().await {
                let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_xray_init"), e)).to_string());
                failed = true;
            }
        }

        if !failed {
            send_progress(&tx, 3, 8, t!("ops.deploy_step_pq"));
            if let Err(e) =
                crate::core::xray::config::ConfigManager::generate_reality_pq_keys().await
            {
                let _ = tx.send(
                    t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_pq"), e))
                        .to_string(),
                );
                failed = true;
            }
        }

        let ip_version = {
            let (v4, v6) = tokio::join!(
                crate::core::system::SystemMonitor::get_public_ip(),
                crate::core::system::SystemMonitor::get_public_ipv6(),
            );
            match (&v4, &v6) {
                (Ok(_), Ok(_)) => crate::core::types::IpVersion::SplitStackV4Primary,
                (Ok(_), Err(_)) => crate::core::types::IpVersion::IPv4,
                (Err(_), Ok(_)) => crate::core::types::IpVersion::IPv6,
                _ => crate::core::types::IpVersion::IPv4,
            }
        };

        if !failed {
            send_progress(
                &tx,
                4,
                8,
                format!("{} ({})", t!("ops.deploy_step_xhttp"), ip_version.label()),
            );
            match crate::core::xray::config::ConfigManager::batch_create_xhttp_reality_enhanced(
                20, ip_version,
            )
            .await
            {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ XHTTP Reality ({}) {} {}",
                                    ip_version.label(),
                                    result.created_count,
                                    result.config_file.as_deref().unwrap_or("?")
                                ),
                                markup: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)).to_string());
                    failed = true;
                }
            }
        }

        if !failed {
            send_progress(
                &tx,
                5,
                8,
                format!("{} ({})", t!("ops.deploy_step_vision"), ip_version.label()),
            );
            match crate::core::xray::config::ConfigManager::batch_create_reality_vision_enhanced(
                20, ip_version,
            )
            .await
            {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ Reality Vision ({}) {} {}",
                                    ip_version.label(),
                                    result.created_count,
                                    result.config_file.as_deref().unwrap_or("?")
                                ),
                                markup: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_vision"), e)).to_string());
                    failed = true;
                }
            }
        }

        if crate::core::singbox::SingBoxInstaller::is_installed().await {
            send_progress(
                &tx,
                6,
                8,
                format!("{} - ⏩ skipped", t!("ops.deploy_step_singbox_init")),
            );
        } else if !failed {
            send_progress(&tx, 6, 8, t!("ops.deploy_step_singbox_init"));
            if let Err(e) = crate::core::singbox::SingBoxInstaller::install().await {
                let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_singbox_init"), e)).to_string());
                failed = true;
            }
        }

        if !failed {
            send_progress(
                &tx,
                7,
                8,
                format!("{} ({})", t!("ops.deploy_step_h2"), ip_version.label()),
            );
            match crate::core::singbox::config::SingBoxConfigManager::batch_create_hysteria2(
                3, ip_version, false, false,
            )
            .await
            {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ Hysteria2 ({}) {} {}",
                                    ip_version.label(),
                                    result.created_count,
                                    result.config_file.as_deref().unwrap_or("?")
                                ),
                                markup: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_h2"), e)).to_string());
                    failed = true;
                }
            }
        }

        if !failed && !all_links.is_empty() {
            let combined = all_links.join("\n\n");
            if let Ok(msg) = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: combined,
                        markup: None,
                    },
                )
                .await
            {
                let adapter_clone = adapter.clone();
                let target_clone = target.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let _ = adapter_clone.delete_message(&target_clone, &msg).await;
                });
            }
        }

        if !failed {
            send_progress(&tx, 8, 8, t!("ops.deploy_step_security"));
            if let Err(e) =
                Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME)
                    .await
            {
                let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_security"), e)).to_string());
            }
        }

        if !failed {
            let _ = tx.send(t!("ops.deploy_success").to_string());
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

fn send_progress(tx: &mpsc::UnboundedSender<String>, step: u8, total: u8, msg: impl Into<String>) {
    let _ = tx.send(format!("[{}/{}] {}", step, total, msg.into()));
}
