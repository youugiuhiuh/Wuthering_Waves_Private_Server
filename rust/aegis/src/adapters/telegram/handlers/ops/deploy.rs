use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::spawn_progress_updater;
use aegis::adapters::common::{MessageContent, TargetId};
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::types::IpVersion;
use rust_i18n::t;
use std::time::Duration;
use teloxide::prelude::*;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("ops.deploy_start"))
        .await?;

    let bot = ctx.bot.clone();
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;
    let adapter = ctx.state.adapter.clone();

    tokio::spawn(async move {
        let (tx, update_task) = spawn_progress_updater(bot.clone(), chat_id, msg_id, |t| {
            format!("🚀 <b>{}</b>\n{}", t!("menu.one_click_deploy"), t)
        });

        let mut failed = false;
        let mut all_links: Vec<String> = Vec::new();
        let target = TargetId(chat_id.0.to_string());

        send_progress(&tx, 1, 8, t!("ops.deploy_step_tune"));
        if MaintenanceManager::tune_vps_generic().await.is_err() {
            let _ = tx.send(t!("ops.deploy_fail", "0" => t!("ops.deploy_fail_tune")).to_string());
            failed = true;
        }

        let xray_installed = tokio::fs::try_exists(aegis::core::paths::xray::BIN)
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
            if let Err(e) =
                aegis::core::xray::installer::RealityInstallerInternal::install_minimal_environment(
                )
                .await
            {
                let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_xray_init"), e)).to_string());
                failed = true;
            }
        }

        if !failed {
            send_progress(&tx, 3, 8, t!("ops.deploy_step_pq"));
            if let Err(e) =
                aegis::core::xray::config::ConfigManager::generate_reality_pq_keys().await
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
                aegis::core::system::SystemMonitor::get_public_ip(),
                aegis::core::system::SystemMonitor::get_public_ipv6(),
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
            match aegis::core::xray::config::ConfigManager::batch_create_xhttp_reality_enhanced(
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
            match aegis::core::xray::config::ConfigManager::batch_create_reality_vision_enhanced(
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
                    let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_vision"), e)).to_string());
                    failed = true;
                }
            }
        }

        if aegis::core::singbox::SingBoxInstaller::is_installed().await {
            send_progress(
                &tx,
                6,
                8,
                format!("{} - ⏩ 已安装，跳过", t!("ops.deploy_step_singbox_init")),
            );
        } else if !failed {
            send_progress(&tx, 6, 8, t!("ops.deploy_step_singbox_init"));
            if let Err(e) = aegis::core::singbox::SingBoxInstaller::install().await {
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
            match aegis::core::singbox::config::SingBoxConfigManager::batch_create_hysteria2(
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
                    if let Err(e) = adapter_clone.delete_message(&target_clone, &msg).await {
                        log::warn!("删除一键部署链接消息失败: {}", e);
                    }
                });
            }
        }

        if !failed {
            send_progress(&tx, 8, 8, t!("ops.deploy_step_security"));
            if let Err(e) =
                aegis::core::system::operations::Operations::perform_maintenance_with_reboot_time(
                    aegis::core::system::operations::Operations::DEFAULT_REBOOT_TIME,
                )
                .await
            {
                let _ = tx.send(t!("ops.deploy_fail", "0" => format!("{}: {}", t!("ops.deploy_fail_security"), e)).to_string());
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

fn send_progress(
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
    step: u8,
    total: u8,
    msg: impl Into<String>,
) {
    let _ = tx.send(format!("[{}/{}] {}", step, total, msg.into()));
}
