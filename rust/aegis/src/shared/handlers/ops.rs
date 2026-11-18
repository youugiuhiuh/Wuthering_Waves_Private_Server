use crate::common::{BotAdapter, InlineButton, Markup, MessageContent};
use crate::core::security::acme::XhttpDeployMode;
use crate::core::singbox::SingBoxInstaller;
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
    adapter: Arc<dyn BotAdapter>,
    target: crate::common::TargetId,
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
            let _ = adapter
                .edit_message(
                    &target,
                    &msg_id,
                    MessageContent {
                        text: title_fn(text),
                        markup: None,
                    },
                )
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    (tx, handle)
}

async fn handle_reload(event: &CallbackEvent) -> HandlerResult {
    let _ = MaintenanceManager::reload_core().await;
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.reload_success").into_owned()),
        )
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_firewall(event: &CallbackEvent) -> HandlerResult {
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.fw_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(adapter, target, msg_id, |t| {
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.upgrade_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();

    tokio::spawn(async move {
        match UpgradeManager::new() {
            Ok(manager) => {
                if let Err(err) = manager.run(adapter.as_ref(), &target).await {
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
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
                        MessageContent {
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.geo_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(adapter.clone(), target.clone(), msg_id, |t| {
                t!("ops.geo_title", "0" => t).to_string()
            });

        let tx_for_adapter = tx.clone();
        let res = MaintenanceManager::update_geodata(move |_, text| {
            let _ = tx_for_adapter.send(text.to_string());
        })
        .await;

        match res {
            Ok(_) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.geo_success").into_owned(),
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
                            text: t!("ops.geo_fail", "0" => e.to_string()).into_owned(),
                            markup: None,
                        },
                    )
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.tune_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(adapter, target, msg_id, |t| {
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.sys_update_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(adapter, target, msg_id, |t| {
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
        .adapter
        .answer_callback(&event.target, &event.callback_id, None)
        .await?;
    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("ops.bbr3_confirm_warn").into_owned(),
                markup: Some(markup),
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

async fn send_bbr3_progress(
    adapter: Arc<dyn BotAdapter>,
    target: crate::common::TargetId,
    msg_id: crate::common::MessageId,
) -> (UnboundedSender<String>, JoinHandle<()>) {
    spawn_progress_updater(adapter, target, msg_id, |t| {
        t!("ops.bbr3_title", "0" => t).to_string()
    })
}

async fn handle_bbr3_install(event: &CallbackEvent) -> HandlerResult {
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.bbr3_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) = send_bbr3_progress(adapter.clone(), target.clone(), msg_id).await;

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
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.bbr3_cancelled").into_owned()),
        )
        .await?;
    Ok(HandlerAction::Redirect("m_ops_center".to_string()))
}

async fn handle_bbr3_reboot_now(event: &CallbackEvent) -> HandlerResult {
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.sys_reboot_text").into_owned()),
        )
        .await?;
    event
        .adapter
        .send_message(
            &event.target,
            MessageContent {
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
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.sys_reboot_later").into_owned()),
        )
        .await?;
    let markup = Markup {
        buttons: vec![vec![InlineButton {
            text: t!("menu.back_net_opt").into(),
            data: "m_net_opt".into(),
        }]],
    };
    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
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
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("ops.sys_reboot_busy").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    }

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.sys_reboot_text").into_owned()),
        )
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
    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    let (tx, _update_task) =
        spawn_progress_updater(adapter.clone(), target.clone(), msg_id.clone(), |t| {
            format!("🚀 <b>{}</b>\n{}", t!("menu.one_click_deploy"), t)
        });

    let mut failed = false;
    let mut all_links: Vec<String> = Vec::new();

    send_progress(&tx, 1, 7, t!("ops.deploy_step_tune"));
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
            7,
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_xray_init")),
        );
    } else if !failed {
        send_progress(&tx, 2, 7, t!("ops.deploy_step_xray_init"));
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
        send_progress(&tx, 3, 7, t!("ops.deploy_step_pq"));
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
        // 步骤 4 的文案必须按 mode 分派：Reality 分支产出的是 XHTTP Reality 配置，
        // TLS 分支产出的是 TLS XHTTP 配置。曾经把 TLS 文案写在 match 之前，
        // 导致无域名的一键部署（Reality）错报成「正在创建 TLS XHTTP」。
        match &mode {
            XhttpDeployMode::Reality => {
                send_progress(
                    &tx,
                    4,
                    7,
                    format!("{} ({})", t!("ops.deploy_step_xhttp"), ip_version.label()),
                );
                match ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version, true).await
                {
                    Ok(result) => {
                        all_links.extend(result.links);
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("ops.deploy_created_xhttp",
                                            "0" => ip_version.label(),
                                            "1" => result.created_count.to_string(),
                                            "2" => result.config_file.as_deref().unwrap_or("?"))
                                    .into_owned(),
                                    markup: None,
                                },
                            )
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
                send_progress(
                    &tx,
                    4,
                    7,
                    format!(
                        "{} ({})",
                        t!("ops.deploy_step_xhttp_tls"),
                        ip_version.label()
                    ),
                );
                match ConfigManager::batch_create_xhttp_tls_enhanced(domain, cert_paths, ip_version)
                    .await
                {
                    Ok(result) => {
                        all_links.extend(result.links);
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("ops.deploy_created_xhttp_tls",
                                            "0" => ip_version.label(),
                                            "1" => result.created_count.to_string(),
                                            "2" => result.config_file.as_deref().unwrap_or("?"))
                                    .into_owned(),
                                    markup: None,
                                },
                            )
                            .await;

                        // Pad with Reality XHTTP to reach 20 total.
                        // prefer_443=false: 443 is already taken by the TLS batch above.
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
                                let _ = adapter
                                    .send_message(
                                        &target,
                                        MessageContent {
                                            text: t!("ops.deploy_created_xhttp_bonus",
                                                    "0" => ip_version.label(),
                                                    "1" => reality_result.created_count.to_string(),
                                                    "2" => reality_result.config_file.as_deref().unwrap_or("?"))
                                            .into_owned(),
                                            markup: None,
                                        },
                                    )
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
            7,
            format!("{} ({})", t!("ops.deploy_step_vision"), ip_version.label()),
        );
        match ConfigManager::batch_create_reality_vision_enhanced(20, ip_version).await {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.deploy_created_vision",
                                    "0" => ip_version.label(),
                                    "1" => result.created_count.to_string(),
                                    "2" => result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                            markup: None,
                        },
                    )
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
            7,
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_singbox_init")),
        );
    } else if !failed {
        send_progress(&tx, 6, 7, t!("ops.deploy_step_singbox_init"));
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

    // ── 已移除：Xray Hysteria2 创建（原步骤 7）────────────────────────────
    // 同一 IP 上 REALITY/XHTTP 与 Hysteria2 共存本身就是可识别特征，反而削弱
    // 抗封锁效果，故一键部署不再创建。手动入口 `u_hy2_batch_*`
    // （`ConfigManager::batch_create_hysteria2_xray`）与其菜单文案保留。

    // ── 已移除：mKCP 创建（原步骤 8 mld / 步骤 9 mlw）────────────────────
    // mKCP 依赖 UDP：被运营商 QoS 时线路直接不可用，且没有 TLS/REALITY 的正常
    // HTTPS 外观，抗主动探测能力弱（Xray 官方亦不建议直接用于过墙）。
    // 手动入口保留：`handle_kcp_ok` → `ConfigManager::batch_create_kcp`。
    // 如需恢复一键部署里的这两步，创建代码需从 git 历史取回。

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
                if let Err(e) = adapter_clone.delete_message(&target_clone, &msg).await {
                    log::warn!("删除一键部署链接消息失败: {}", e);
                }
            });
        }
    }

    if !failed {
        send_progress(&tx, 7, 7, t!("ops.deploy_step_security"));
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
        let _ = adapter
            .send_message(
                &target,
                MessageContent {
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

    /// `run_one_click` 的可执行代码（剔除注释行）。
    ///
    /// 剔除注释是必要的：移除说明与文档里会出现待测的 key 名 / 函数名，
    /// 不剔除就会误计为真实引用。
    fn one_click_code() -> String {
        let src = include_str!("ops.rs");
        let start = src
            .find("pub async fn run_one_click")
            .expect("run_one_click 应存在");
        let body = &src[start..];
        let end = body
            .find("\nasync fn handle_one_click")
            .expect("one_click_code 找不到 handle_one_click 边界");
        body[..end]
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 回归：一键部署的进度步数与总步数必须与实现里真正发送的一致（当前 1→7：
    /// 1 调优、2 Xray 初始化、3 PQ 密钥、4 XHTTP、5 Vision、6 sing-box 初始化、
    /// 7 安全维护）。曾经用一个没人读的常量去「测」步数，把某一步改回 `/8` 时
    /// 测试依旧全绿，而 Telegram 里的进度会跳。
    #[test]
    fn one_click_progress_totals_are_seven() {
        let calls = send_progress_calls(&one_click_code());

        assert!(
            !calls.is_empty(),
            "run_one_click 应至少调用一次 send_progress"
        );
        for (step, total) in &calls {
            assert_eq!(*total, 7, "步骤 {step} 的总步数必须是 7");
        }
        let mut steps: Vec<u8> = calls.iter().map(|(s, _)| *s).collect();
        steps.dedup();
        assert_eq!(
            steps,
            (1..=7).collect::<Vec<u8>>(),
            "步骤号必须是 1→7，不得跳号"
        );
        assert_eq!(calls.last().map(|(s, _)| *s), Some(7), "最后一步应为 7/7");
    }

    /// 取出所有 `send_progress(&tx, 步号, 总步数, ...)` 的 `(步号, 总步数)`。
    ///
    /// 只取消息参数之前的两个数字：`t!(...)` / `format!(...)` 之后出现的数字
    /// 属于文案（例如标题键里带数字），不能当成步号；也不用全局字符串搜索 ——
    /// 否则代码里任何 `foo(3, 8, x)` 都会被误判成「总步数是 8」。
    fn send_progress_calls(code: &str) -> Vec<(u8, u8)> {
        code.split("send_progress(")
            .skip(1)
            .filter_map(|seg| {
                let args = ["t!(", "format!("]
                    .iter()
                    .filter_map(|m| seg.find(m))
                    .min()
                    .map_or(seg, |at| &seg[..at]);
                let mut nums = args
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse::<u8>().ok());
                Some((nums.next()??, nums.next()??))
            })
            .collect()
    }

    /// 回归：一键部署不得再创建 Xray Hysteria2 配置。
    ///
    /// 单独关闭一个协议比堆叠协议更耐封锁：REALITY/XHTTP 与 Hysteria2 共用同一
    /// 台主机的同一个 IP 时，共存本身就是可识别的特征。
    #[test]
    fn one_click_does_not_create_xray_hy2() {
        let code = one_click_code();

        assert!(
            !code.contains("batch_create_hysteria2"),
            "一键部署不应调用 batch_create_hysteria2_xray"
        );
        for key in ["deploy_step_h2", "deploy_fail_h2", "deploy_created_h2"] {
            assert!(!code.contains(key), "一键部署不应引用 i18n 键 {key}");
        }
    }

    /// 回归：一键部署不得再引用任何 mKCP 创建入口与相关 i18n 键。
    /// 回归：一键部署不得再创建 mKCP 配置。
    ///
    /// 相关的 6 个 `ops.deploy_*kcp_*` 键已全部删除，因此无需再断言键名。
    #[test]
    fn one_click_does_not_create_mkcp() {
        let code = one_click_code();

        assert!(
            !code.contains("batch_create_kcp"),
            "一键部署不应调用 batch_create_kcp"
        );
    }

    /// 取出 `run_one_click` 的可执行代码，并裁到 `match &mode` 为止。
    ///
    /// 返回 `(match 之前的代码, match 之后的代码)`。
    fn one_click_mode_split() -> (String, String) {
        let code = one_click_code();
        let (before, after) = code
            .split_once("match &mode")
            .expect("run_one_click 应包含 match &mode 分支");
        (before.to_string(), after.to_string())
    }

    /// 回归：步骤 4 的进度文案必须按 `XhttpDeployMode` 分派。
    ///
    /// 曾经的问题：`send_progress(4, 8, deploy_step_xhttp_tls)` 写在 `match &mode`
    /// **之前**，无域名的一键部署（走 `XhttpDeployMode::Reality`）会错报成
    /// 「正在创建 TLS XHTTP」，而实际产出的是 Reality XHTTP 配置。
    ///
    /// 断言的是**位置**而不只是存在性：把文案 key 移到 match 之外、
    /// 让两个分支共用同一文案，都必须让本测试失败。
    #[test]
    fn one_click_step4_progress_text_matches_mode() {
        let (before_mode, after_mode) = one_click_mode_split();

        // 两个分支各自的步骤 4 文案，都必须出现在 match 之后（即分支内部）。
        assert!(
            after_mode.contains("ops.deploy_step_xhttp\""),
            "Reality 分支必须有专属的步骤 4 文案（ops.deploy_step_xhttp）"
        );
        assert!(
            after_mode.contains("ops.deploy_step_xhttp_tls"),
            "TLS 分支必须有专属的步骤 4 文案（ops.deploy_step_xhttp_tls）"
        );

        // 反向断言：match 之前不得出现任何步骤 4 的文案 key。
        // 这正是原 bug 的形态 —— 无条件发送一次 TLS 文案，再进分支。
        for key in ["ops.deploy_step_xhttp\"", "ops.deploy_step_xhttp_tls"] {
            assert!(
                !before_mode.contains(key),
                "步骤 4 文案 {key} 不得写在 match &mode 之前，否则会与分支脱钩"
            );
        }

        // 两个 key 在分支内必须分属不同分支：Reality key 先于 TLS key 出现，
        // 且中间隔着分支边界。若两者被塞进同一分支，这里会失败。
        let reality_at = after_mode
            .find("ops.deploy_step_xhttp\"")
            .expect("Reality key 应存在");
        let tls_at = after_mode
            .find("ops.deploy_step_xhttp_tls")
            .expect("TLS key 应存在");
        assert!(
            reality_at < tls_at,
            "Reality 分支应先于 TLS 分支定义各自的步骤 4 文案"
        );
    }
}
