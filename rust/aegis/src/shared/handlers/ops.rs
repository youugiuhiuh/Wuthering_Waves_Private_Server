use crate::app::interaction::{ConversationId, OutputAction, OutputPayload, Sensitivity};
use crate::common::{InlineButton, Markup};
use crate::core::security::acme::XhttpDeployMode;
use crate::core::singbox::SingBoxInstaller;
use crate::core::singbox::config::SingBoxConfigManager;

use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::system::operations::{Operations, REBOOT_FLAG};
use crate::core::system::upgrade::UpgradeManager;
use crate::core::types::IpVersion;
use crate::core::xray::config::ConfigManager;
use crate::core::xray::installer::RealityInstallerInternal;
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    match event.data.as_str() {
        "a_reload" => handle_reload(event).await,
        "a_fw" => handle_firewall(event).await,
        "a_upgrade" => handle_upgrade(event).await,
        "a_geo" => handle_geo(event).await,
        "a_tune" => handle_tune(event).await,
        "a_sys_update" => handle_sys_update(event).await,
        "a_bbr3" => handle_bbr3_prompt(event).await,
        "a_bbr3_confirm" => handle_bbr3_install(event).await,
        "a_bbr3_cancel" => handle_bbr3_cancel(event).await,
        "a_bbr3_reboot_now" => handle_bbr3_reboot_now(event).await,
        "a_bbr3_reboot_later" => handle_bbr3_reboot_later(event).await,
        "a_sys_reboot" => handle_sys_reboot(event).await,
        "a_one_click" => handle_one_click(event).await,
        _ => Ok(HandlerAction::Done),
    }
}

fn spawn_progress_updater(
    output: Arc<dyn crate::app::output::BusinessOutput>,
    conversation_id: ConversationId,
    msg_id: crate::common::MessageId,
    title_fn: impl Fn(String) -> String + Send + 'static,
) -> (UnboundedSender<String>, JoinHandle<()>) {
    let (tx, mut rx) = unbounded_channel::<String>();
    let handle = tokio::spawn(async move {
        let mut last = String::new();
        while let Some(text) = rx.recv().await {
            if text == last {
                continue;
            }
            last = text.clone();
            let _ = output
                .publish(OutputAction::Edit {
                    target_conversation: conversation_id.clone(),
                    message_id: msg_id.0.clone(),
                    payload: OutputPayload::Text {
                        text: title_fn(text),
                    },
                })
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    (tx, handle)
}

async fn handle_reload(event: &CallbackEvent) -> HandlerResult {
    let _ = MaintenanceManager::reload_core().await;
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.reload_success").into_owned()),
        })
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_firewall(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.fw_start").into_owned()),
        })
        .await?;

    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(output.clone(), conversation_id.clone(), msg_id, |t| {
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
            Ok(Err(err)) => {
                let _ = tx.send(t!("ops.fw_fail", "0" => err.to_string()).to_string());
            }
            Err(_) => {
                let _ = tx.send(t!("ops.fw_timeout").to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

async fn handle_upgrade(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.upgrade_start").into_owned()),
        })
        .await?;

    let adapter = event.output.as_adapter().clone();
    let target = event.target.clone();

    tokio::spawn(async move {
        match UpgradeManager::new() {
            Ok(manager) => {
                let reporter = crate::shared::reporters::StatusMessageReporter::new(
                    adapter.clone(),
                    target.clone(),
                );
                if let Err(err) = manager.run(&reporter).await {
                    let _ = adapter
                        .send_message(
                            &target,
                            crate::common::MessageContent {
                                text: t!("ops.upgrade_fail", "0" => err.to_string()).into_owned(),
                                markup: None,
                            },
                        )
                        .await;
                }
            }
            Err(err) => {
                let _ = adapter
                    .send_message(
                        &target,
                        crate::common::MessageContent {
                            text: t!("ops.upgrade_init_fail", "0" => err.to_string()).into_owned(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    });

    Ok(HandlerAction::Done)
}

async fn handle_geo(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.geo_start").into_owned()),
        })
        .await?;

    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(output.clone(), conversation_id.clone(), msg_id, |t| {
                t!("ops.geo_title", "0" => t).to_string()
            });

        let tx_for_output = tx.clone();
        let res = MaintenanceManager::update_geodata(move |_, text| {
            let _ = tx_for_output.send(text.to_string());
        })
        .await;

        match res {
            Ok(_) => {
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.geo_success").into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
            Err(e) => {
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.geo_fail", "0" => e.to_string()).into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

async fn handle_tune(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.tune_start").into_owned()),
        })
        .await?;

    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(output.clone(), conversation_id.clone(), msg_id, |t| {
                format!("⚙️ <b>{}</b>\n{}", t!("menu.generic_tune"), t)
            });

        let result = MaintenanceManager::tune_vps_generic().await;
        match result {
            Ok(()) => {
                let _ = tx.send(t!("ops.tune_success").to_string());
            }
            Err(e) => {
                let _ = tx.send(t!("ops.tune_fail", "0" => e.to_string()).to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

async fn handle_sys_update(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.sys_update_start").into_owned()),
        })
        .await?;

    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(output.clone(), conversation_id.clone(), msg_id, |t| {
                format!("⬆️ <b>{}</b>\n{}", t!("menu.sys_cmd"), t)
            });

        let tx_clone = tx.clone();
        let result = MaintenanceManager::upgrade_system_packages(move |text| {
            let _ = tx_clone.send(text.to_string());
        })
        .await;

        match result {
            Ok(()) => {
                let _ = tx.send(t!("ops.sys_update_success").to_string());
            }
            Err(e) => {
                let _ = tx.send(t!("ops.sys_update_fail", "0" => e.to_string()).to_string());
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

async fn handle_bbr3_prompt(event: &CallbackEvent) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("ops.bbr3_confirm_btn").into(),
                data: "a_bbr3_confirm".into(),
            }],
            vec![InlineButton {
                text: t!("ops.bbr3_cancel").into(),
                data: "a_bbr3_cancel".into(),
            }],
        ],
    };
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: None,
        })
        .await?;
    event
        .output
        .as_adapter()
        .edit_message(
            &event.target,
            &event.msg_id,
            crate::common::MessageContent {
                text: t!("ops.bbr3_confirm_warn").into_owned(),
                markup: Some(markup),
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

async fn send_bbr3_progress(
    output: Arc<dyn crate::app::output::BusinessOutput>,
    conversation_id: ConversationId,
    msg_id: crate::common::MessageId,
) -> (UnboundedSender<String>, JoinHandle<()>) {
    spawn_progress_updater(output, conversation_id, msg_id, |t| {
        t!("ops.bbr3_title", "0" => t).to_string()
    })
}

async fn handle_bbr3_install(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.bbr3_start").into_owned()),
        })
        .await?;

    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let msg_id = event.msg_id.clone();
    let target = event.target.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            send_bbr3_progress(output.clone(), conversation_id.clone(), msg_id).await;

        let tx_clone = tx.clone();
        let res = tokio::time::timeout(
            Duration::from_secs(300),
            MaintenanceManager::install_bbr3(move |desc| {
                let _ = tx_clone.send(desc.to_string());
            }),
        )
        .await;

        match res {
            Ok(Ok(status)) => {
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

                if status.reboot_required {
                    let markup = Markup {
                        buttons: vec![
                            vec![InlineButton {
                                text: t!("ops.bbr3_reboot_now").into(),
                                data: "a_bbr3_reboot_now".into(),
                            }],
                            vec![InlineButton {
                                text: t!("ops.bbr3_reboot_later").into(),
                                data: "a_bbr3_reboot_later".into(),
                            }],
                        ],
                    };
                    let _ = output
                        .as_adapter()
                        .send_message(
                            &target,
                            crate::common::MessageContent {
                                text: t!("ops.bbr3_reboot_prompt").into_owned(),
                                markup: Some(markup),
                            },
                        )
                        .await;
                }
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
    });

    Ok(HandlerAction::Done)
}

async fn handle_bbr3_cancel(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.bbr3_cancelled").into_owned()),
        })
        .await?;
    Ok(HandlerAction::Redirect("m_ops_center".to_string()))
}

async fn handle_bbr3_reboot_now(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.sys_reboot_text").into_owned()),
        })
        .await?;
    event
        .output
        .as_adapter()
        .send_message(
            &event.target,
            crate::common::MessageContent {
                text: t!("ops.bbr3_reboot_now_msg").into_owned(),
                markup: None,
            },
        )
        .await?;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = Operations::reboot_system().await;
    });
    Ok(HandlerAction::Done)
}

async fn handle_bbr3_reboot_later(event: &CallbackEvent) -> HandlerResult {
    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.sys_reboot_later").into_owned()),
        })
        .await?;
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_net_opt").into(),
            data: "m_net_opt".into(),
        }]],
    };
    event
        .output
        .as_adapter()
        .edit_message(
            &event.target,
            &event.msg_id,
            crate::common::MessageContent {
                text: t!("ops.bbr3_reboot_later_msg").into_owned(),
                markup: Some(markup),
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_sys_reboot(event: &CallbackEvent) -> HandlerResult {
    if REBOOT_FLAG.load(std::sync::atomic::Ordering::SeqCst) {
        event
            .output
            .publish(OutputAction::AnswerCallback {
                callback_id: event.callback_id.clone(),
                text: Some(t!("ops.sys_reboot_busy").into_owned()),
            })
            .await?;
        return Ok(HandlerAction::Done);
    }

    event
        .output
        .publish(OutputAction::AnswerCallback {
            callback_id: event.callback_id.clone(),
            text: Some(t!("ops.sys_reboot_text").into_owned()),
        })
        .await?;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = Operations::reboot_system().await;
    });
    Ok(HandlerAction::Done)
}

pub async fn run_one_click(
    event: CallbackEvent,
    _state: (),
    mode: XhttpDeployMode,
) -> anyhow::Result<()> {
    let output = event.output.clone();
    let conversation_id = event.origin.conversation_id.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    let (tx, _update_task) = spawn_progress_updater(
        output.clone(),
        conversation_id.clone(),
        msg_id.clone(),
        |t| format!("🚀 <b>{}</b>\n{}", t!("menu.one_click_deploy"), t),
    );

    let mut failed = false;
    let mut all_links: Vec<String> = Vec::new();

    send_progress(&tx, 1, 10, t!("ops.deploy_step_tune"));
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
            10,
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_xray_init")),
        );
    } else if !failed {
        send_progress(&tx, 2, 10, t!("ops.deploy_step_xray_init"));
        if let Err(e) = RealityInstallerInternal::install_minimal_environment().await {
            let _ = tx.send(
                t!("ops.deploy_fail",
                    "0" => format!("{}: {}", t!("ops.deploy_fail_xray_init"), e)
                )
                .to_string(),
            );
            failed = true;
        }
    }

    if !failed {
        send_progress(&tx, 3, 10, t!("ops.deploy_step_pq"));
        if let Err(e) = ConfigManager::generate_reality_pq_keys().await {
            let _ = tx.send(
                t!("ops.deploy_fail",
                    "0" => format!("{}: {}", t!("ops.deploy_fail_pq"), e)
                )
                .to_string(),
            );
            failed = true;
        }
    }

    let ip_version = {
        let (v4, v6) = tokio::join!(
            SystemMonitor::get_public_ip(),
            SystemMonitor::get_public_ipv6(),
        );
        match (&v4, &v6) {
            (Ok(_), Ok(_)) => IpVersion::SplitStackV4Primary,
            (Ok(_), Err(_)) => IpVersion::IPv4,
            (Err(_), Ok(_)) => IpVersion::IPv6,
            _ => IpVersion::IPv4,
        }
    };

    if !failed {
        send_progress(
            &tx,
            4,
            10,
            format!(
                "{} ({})",
                t!("ops.deploy_step_xhttp_tls"),
                ip_version.label()
            ),
        );
        match &mode {
            XhttpDeployMode::Reality => {
                match ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version, true).await
                {
                    Ok(result) => {
                        all_links.extend(result.links);
                        let _ = output
                            .publish(OutputAction::SendText {
                                target_conversation: conversation_id.clone(),
                                payload: OutputPayload::Text {
                                    text: t!("ops.deploy_created_xhttp",
                                            "0" => ip_version.label(),
                                            "1" => result.created_count.to_string(),
                                            "2" => result.config_file.as_deref().unwrap_or("?"))
                                    .into_owned(),
                                },
                                sensitivity: Sensitivity::Public,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx.send(
                            t!("ops.deploy_fail",
                                "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)
                            )
                            .to_string(),
                        );
                        failed = true;
                    }
                }
            }
            XhttpDeployMode::Tls { domain, cert_paths } => {
                match ConfigManager::batch_create_xhttp_tls_enhanced(domain, cert_paths, ip_version)
                    .await
                {
                    Ok(result) => {
                        all_links.extend(result.links);
                        let _ = output
                            .publish(OutputAction::SendText {
                                target_conversation: conversation_id.clone(),
                                payload: OutputPayload::Text {
                                    text: t!("ops.deploy_created_xhttp_tls",
                                            "0" => ip_version.label(),
                                            "1" => result.created_count.to_string(),
                                            "2" => result.config_file.as_deref().unwrap_or("?"))
                                    .into_owned(),
                                },
                                sensitivity: Sensitivity::Public,
                            })
                            .await;

                        let reality_count = 20_usize.saturating_sub(result.created_count);
                        match ConfigManager::batch_create_xhttp_reality_enhanced(
                            reality_count,
                            ip_version,
                            false,
                        )
                        .await
                        {
                            Ok(reality_result) => {
                                all_links.extend(reality_result.links);
                                let _ = output
                                    .publish(OutputAction::SendText {
                                        target_conversation: conversation_id.clone(),
                                        payload: OutputPayload::Text {
                                            text: t!("ops.deploy_created_xhttp_bonus",
                                                    "0" => ip_version.label(),
                                                    "1" => reality_result.created_count.to_string(),
                                                    "2" => reality_result.config_file.as_deref().unwrap_or("?"))
                                            .into_owned(),
                                        },
                                        sensitivity: Sensitivity::Public,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = tx.send(
                                    t!("ops.deploy_fail",
                                        "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)
                                    )
                                    .to_string(),
                                );
                                failed = true;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(
                            t!("ops.deploy_fail",
                                "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp_tls"), e)
                            )
                            .to_string(),
                        );
                        failed = true;
                    }
                }
            }
        }
    }

    if !failed {
        send_progress(
            &tx,
            5,
            10,
            format!("{} ({})", t!("ops.deploy_step_vision"), ip_version.label()),
        );
        match ConfigManager::batch_create_reality_vision_enhanced(20, ip_version).await {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.deploy_created_vision",
                                    "0" => ip_version.label(),
                                    "1" => result.created_count.to_string(),
                                    "2" => result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(
                    t!("ops.deploy_fail",
                        "0" => format!("{}: {}", t!("ops.deploy_fail_vision"), e)
                    )
                    .to_string(),
                );
                failed = true;
            }
        }
    }

    if SingBoxInstaller::is_installed().await {
        send_progress(
            &tx,
            6,
            10,
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_singbox_init")),
        );
    } else if !failed {
        send_progress(&tx, 6, 10, t!("ops.deploy_step_singbox_init"));
        if let Err(e) = SingBoxInstaller::install().await {
            let _ = tx.send(
                t!("ops.deploy_fail",
                    "0" => format!("{}: {}", t!("ops.deploy_fail_singbox_init"), e)
                )
                .to_string(),
            );
            failed = true;
        }
    }

    if !failed {
        send_progress(
            &tx,
            7,
            10,
            format!("{} ({})", t!("ops.deploy_step_h2"), ip_version.label()),
        );
        match SingBoxConfigManager::batch_create_hysteria2(3, ip_version, false, false).await {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.deploy_created_h2",
                                    "0" => ip_version.label(),
                                    "1" => result.created_count.to_string(),
                                    "2" => result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(
                    t!("ops.deploy_fail",
                        "0" => format!("{}: {}", t!("ops.deploy_fail_h2"), e)
                    )
                    .to_string(),
                );
                failed = true;
            }
        }
    }

    if !failed {
        let _ = output
            .as_adapter()
            .send_message(
                &target,
                crate::common::MessageContent {
                    text: t!("ops.deploy_step_kcp_dns").into_owned(),
                    markup: None,
                },
            )
            .await;
    }

    if !failed {
        send_progress(&tx, 8, 10, t!("ops.deploy_step_kcp_dns"));
        match ConfigManager::batch_create_kcp(5, ip_version, &["mld"]).await {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.deploy_created_kcp_dns",
                                    "0" => result.created_count.to_string(),
                                    "1" => result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(
                    t!("ops.deploy_fail",
                        "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_dns"), e)
                    )
                    .to_string(),
                );
                failed = true;
            }
        }
    }

    if !failed {
        send_progress(&tx, 9, 10, t!("ops.deploy_step_kcp_wechat"));
        match ConfigManager::batch_create_kcp(5, ip_version, &["mlw"]).await {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = output
                    .publish(OutputAction::SendText {
                        target_conversation: conversation_id.clone(),
                        payload: OutputPayload::Text {
                            text: t!("ops.deploy_created_kcp_wechat",
                                    "0" => result.created_count.to_string(),
                                    "1" => result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                        },
                        sensitivity: Sensitivity::Public,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(
                    t!("ops.deploy_fail",
                        "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_wechat"), e)
                    )
                    .to_string(),
                );
                failed = true;
            }
        }
    }

    if !failed && !all_links.is_empty() {
        let combined = all_links.join("\n\n");
        if let Ok(msg) = output
            .as_adapter()
            .send_message(
                &target,
                crate::common::MessageContent {
                    text: combined,
                    markup: None,
                },
            )
            .await
        {
            let adapter_clone = output.as_adapter().clone();
            let target_clone = target.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if let Err(e) = adapter_clone.delete_message(&target_clone, &msg).await {
                    log::warn!("删除一键部署链接消息失败: {}", e);
                }
            });
        }
    }

    if !failed {
        send_progress(&tx, 10, 10, t!("ops.deploy_step_security"));
        if let Err(e) =
            Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME).await
        {
            let _ = tx.send(
                t!("ops.deploy_fail",
                    "0" => format!("{}: {}", t!("ops.deploy_fail_security"), e)
                )
                .to_string(),
            );
            failed = true;
        }
    }

    if !failed {
        let _ = output
            .as_adapter()
            .send_message(
                &target,
                crate::common::MessageContent {
                    text: t!("ops.deploy_success").into_owned(),
                    markup: None,
                },
            )
            .await;
    }

    Ok(())
}

async fn handle_one_click(event: &CallbackEvent) -> HandlerResult {
    crate::shared::handlers::xray::show_domain_choice(
        event,
        crate::core::types::DomainFlowSource::OneClick,
    )
    .await
}

fn send_progress(tx: &UnboundedSender<String>, step: u8, total: u8, msg: impl Into<String>) {
    let _ = tx.send(format!("[{}/{}] {}", step, total, msg.into()));
}

#[cfg(test)]
mod tests {
    use crate::core::security::acme::{CertPaths, XhttpDeployMode};
    use std::path::PathBuf;

    fn tls_mode(domain: &str) -> XhttpDeployMode {
        XhttpDeployMode::Tls {
            domain: domain.to_string(),
            cert_paths: CertPaths {
                fullchain: PathBuf::from("/fake/fullchain"),
                privkey: PathBuf::from("/fake/privkey"),
            },
        }
    }

    fn xhttp_mode_for_one_click(has_domain: bool, mode: XhttpDeployMode) -> XhttpDeployMode {
        if has_domain {
            mode
        } else {
            XhttpDeployMode::Reality
        }
    }

    #[test]
    fn one_click_selects_only_xhttp_backend() {
        assert!(matches!(
            xhttp_mode_for_one_click(true, tls_mode("example.com")),
            XhttpDeployMode::Tls { .. }
        ));
    }

    #[test]
    fn one_click_without_domain_selects_reality() {
        assert!(matches!(
            xhttp_mode_for_one_click(false, tls_mode("example.com")),
            XhttpDeployMode::Reality
        ));
    }
}
