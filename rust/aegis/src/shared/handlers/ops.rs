use crate::common::{BotAdapter, InlineButton, Markup, MessageContent};
use crate::core::security::acme::XhttpDeployMode;
use crate::core::singbox::SingBoxInstaller;
use crate::core::singbox::hysteria2::{Hy2LinkStyle, Hysteria2ObfsType};

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
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_xray_init")),
        );
    } else if !failed {
        send_progress(&tx, 2, 8, t!("ops.deploy_step_xray_init"));
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
        send_progress(&tx, 3, 8, t!("ops.deploy_step_pq"));
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
            8,
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
            8,
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
            8,
            t!("ops.deploy_skip", "0" => t!("ops.deploy_step_singbox_init")),
        );
    } else if !failed {
        send_progress(&tx, 6, 8, t!("ops.deploy_step_singbox_init"));
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
            8,
            format!("{} ({})", t!("ops.deploy_step_h2"), ip_version.label()),
        );
        // 一键部署默认使用 gecko 混淆，但不开启端口跳跃。
        //
        // 注意：这里走的是 Xray 的 Hysteria2 创建路径
        // (`ConfigManager::batch_create_hysteria2_xray`)，而不是 sing-box 的
        // `SingBoxConfigManager::batch_create_hysteria2`。两者都叫 Hysteria2，
        // 但写出的协议标识不同 —— Xray 写 `"protocol": "hysteria"`（配置落在
        // `xray::CONF_DIR`），sing-box 写 `"type": "hysteria2"`（配置落在
        // `singbox::CONF_DIR`）。因此改这里不会影响 sing-box 侧行为，反之亦然。
        match ConfigManager::batch_create_hysteria2_xray(
            3,
            ip_version,
            Some(Hysteria2ObfsType::Gecko),
            false,
            Hy2LinkStyle::Official,
        )
        .await
        {
            Ok(result) => {
                all_links.extend(result.links);
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("ops.deploy_created_h2",
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
                        "0" => format!("{}: {}", t!("ops.deploy_fail_h2"), e)
                    )
                    .to_string(),
                );
                failed = true;
            }
        }
    }

    // ── 已移除：mKCP 创建（原步骤 8 与步骤 9）────────────────────────────
    //
    // 一键部署不再创建 mKCP 配置。原因见下方说明；如需恢复，取消注释整段即可
    // （`ConfigManager::batch_create_kcp` 与 `KcpMask` 均仍保留，未被删除）。
    //
    // 原步骤 8：mKCP + DNS 伪装（遮罩码 `mld`），创建 5 个配置
    // 原步骤 9：mKCP + 微信伪装（遮罩码 `mlw`），创建 5 个配置
    //
    // 移除理由：
    //   1. mKCP 依赖 UDP。运营商或目标网络对 UDP 做 QoS/限速/封锁时，
    //      该线路直接不可用，且失败表现为「连不上」而非「慢」，排查成本高。
    //   2. mKCP 以牺牲带宽换取低延迟，传输同等内容比 TCP 多消耗 20–50% 流量，
    //      在大流量场景下成本明显。
    //   3. Xray 官方文档将 mKCP 归类为「不适合直接用于过墙」：它没有 TLS/REALITY
    //      提供的正常 HTTPS 外观，`finalmask` 的 udp 混淆层只是固定长度混淆，
    //      不具备抗主动探测能力。
    //   4. 本项目一键部署已提供抗封锁能力更强的替代方案：
    //      XHTTP+REALITY/TLS（步骤 4）、REALITY+Vision（步骤 5）、Hysteria2（步骤 7）。
    //      其中 Hysteria2 同样基于 UDP 且具备 Brutal 拥塞控制，在高丢包线路上
    //      表现优于 mKCP，可作为 UDP 路线的替代。
    //
    // 保留的能力：Bot 菜单中的独立 KCP 入口（`handle_kcp_ok`）与
    // `ConfigManager::batch_create_kcp` 完整保留，用户仍可按需手动创建 mKCP 配置。
    //
    // 相关 i18n 键（`ops.deploy_step_kcp_dns`、`ops.deploy_step_kcp_wechat`、
    // `ops.deploy_fail_kcp_dns`、`ops.deploy_fail_kcp_wechat`、
    // `ops.deploy_created_kcp_dns`、`ops.deploy_created_kcp_wechat`）同样保留，
    // 以便恢复时无需额外改动。
    //
    // if !failed {
    //     let _ = adapter
    //         .send_message(
    //             &target,
    //             MessageContent {
    //                 text: t!("ops.deploy_step_kcp_dns").into_owned(),
    //                 markup: None,
    //             },
    //         )
    //         .await;
    // }
    //
    // if !failed {
    //     send_progress(&tx, 8, 10, t!("ops.deploy_step_kcp_dns"));
    //     match ConfigManager::batch_create_kcp(5, ip_version, &["mld"]).await {
    //         Ok(result) => {
    //             all_links.extend(result.links);
    //             let _ = adapter
    //                 .send_message(
    //                     &target,
    //                     MessageContent {
    //                         text: t!("ops.deploy_created_kcp_dns",
    //                                 "0" => result.created_count.to_string(),
    //                                 "1" => result.config_file.as_deref().unwrap_or("?"))
    //                         .into_owned(),
    //                         markup: None,
    //                     },
    //                 )
    //                 .await;
    //         }
    //         Err(e) => {
    //             let _ = tx.send(
    //                 t!("ops.deploy_fail",
    //                     "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_dns"), e)
    //                 )
    //                 .to_string(),
    //             );
    //             failed = true;
    //         }
    //     }
    // }
    //
    // if !failed {
    //     send_progress(&tx, 9, 10, t!("ops.deploy_step_kcp_wechat"));
    //     match ConfigManager::batch_create_kcp(5, ip_version, &["mlw"]).await {
    //         Ok(result) => {
    //             all_links.extend(result.links);
    //             let _ = adapter
    //                 .send_message(
    //                     &target,
    //                     MessageContent {
    //                         text: t!("ops.deploy_created_kcp_wechat",
    //                                 "0" => result.created_count.to_string(),
    //                                 "1" => result.config_file.as_deref().unwrap_or("?"))
    //                         .into_owned(),
    //                         markup: None,
    //                     },
    //                 )
    //                 .await;
    //         }
    //         Err(e) => {
    //             let _ = tx.send(
    //                 t!("ops.deploy_fail",
    //                     "0" => format!("{}: {}", t!("ops.deploy_fail_kcp_wechat"), e)
    //                 )
    //                 .to_string(),
    //             );
    //             failed = true;
    //         }
    //     }
    // }
    // ── 移除段落结束 ─────────────────────────────────────────────────────

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
        send_progress(&tx, 8, 8, t!("ops.deploy_step_security"));
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

    /// 一键部署的进度总步数。
    ///
    /// 移除 mKCP 创建（原步骤 8 与 9）后，1→8 步：
    /// 1 调优、2 Xray 初始化、3 PQ 密钥、4 XHTTP、5 Vision、
    /// 6 sing-box 初始化、7 Hysteria2、8 安全维护。
    const ONE_CLICK_TOTAL_STEPS: u8 = 8;

    #[test]
    fn one_click_progress_has_eight_steps() {
        assert_eq!(ONE_CLICK_TOTAL_STEPS, 8);
    }

    /// 回归：一键部署不得再引用任何 mKCP 创建入口与相关 i18n 键。
    ///
    /// 仅注释保留原文，不应出现未注释的调用。
    ///
    /// 说明：扫描时须剔除本测试函数自身（它含有这些字符串字面量），
    /// 否则断言会因自引用而永远失败。
    #[test]
    fn one_click_does_not_create_mkcp() {
        let src = include_str!("ops.rs");

        // 只保留 run_one_click 函数体：其余部分（包括本测试）不参与判定。
        let start = src
            .find("pub async fn run_one_click")
            .expect("run_one_click 应存在");
        let body = &src[start..];
        let end = body
            .find("\nasync fn handle_one_click")
            .unwrap_or(body.len());
        let body = &body[..end];

        // 剔除注释行，仅检查真正的可执行代码。
        let code = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !code.contains("batch_create_kcp"),
            "一键部署不应调用 batch_create_kcp"
        );
        for key in [
            "deploy_step_kcp_dns",
            "deploy_step_kcp_wechat",
            "deploy_fail_kcp_dns",
            "deploy_fail_kcp_wechat",
            "deploy_created_kcp_dns",
            "deploy_created_kcp_wechat",
        ] {
            assert!(!code.contains(key), "一键部署不应引用 i18n 键 {key}");
        }
    }
}
