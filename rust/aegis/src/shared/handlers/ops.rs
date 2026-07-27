use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent};
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
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::oneshot;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

static ONE_CLICK_IP_PENDING: LazyLock<Mutex<HashMap<String, oneshot::Sender<IpVersion>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
        d if d.starts_with("a_one_click_ip:") => handle_one_click_ip_response(event, d).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_one_click_ip_response(event: &CallbackEvent, data: &str) -> HandlerResult {
    let _ = (event, data);
    Ok(HandlerAction::Done)
}

fn spawn_progress_updater(
    adapter: Arc<dyn BotAdapter>,
    target: crate::adapters::common::TargetId,
    msg_id: crate::adapters::common::MessageId,
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
    target: crate::adapters::common::TargetId,
    msg_id: crate::adapters::common::MessageId,
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

async fn handle_one_click(event: &CallbackEvent) -> HandlerResult {
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("ops.deploy_start").into_owned()),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();
    let msg_id = event.msg_id.clone();

    tokio::spawn(async move {
        let (tx, update_task) =
            spawn_progress_updater(adapter.clone(), target.clone(), msg_id, |t| {
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
                format!("{} - ⏩ 已安装，跳过", t!("ops.deploy_step_xray_init")),
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
                format!("{} ({})", t!("ops.deploy_step_xhttp"), ip_version.label()),
            );
            match ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version).await {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ XHTTP Reality ({}) 已创建 {} 个配置\n📁 {}",
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
                                text: format!(
                                    "✅ Reality Vision ({}) 已创建 {} 个配置\n📁 {}",
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
                format!("{} - ⏩ 已安装，跳过", t!("ops.deploy_step_singbox_init")),
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
            match SingBoxConfigManager::batch_create_hysteria2(3, ip_version, false, false).await {
                Ok(result) => {
                    all_links.extend(result.links);
                    let _ = adapter
                        .send_message(
                            &target,
                            MessageContent {
                                text: format!(
                                    "✅ Hysteria2 ({}) 已创建 {} 个配置\n📁 {}",
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
                Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME)
                    .await
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
            let _ = tx.send(t!("ops.deploy_success").to_string());
        }

        drop(tx);
        let _ = update_task.await;
    });

    Ok(HandlerAction::Done)
}

fn send_progress(tx: &UnboundedSender<String>, step: u8, total: u8, msg: impl Into<String>) {
    let _ = tx.send(format!("[{}/{}] {}", step, total, msg.into()));
}
