use std::sync::Arc;

use tokio::time::{Duration, sleep};

use crate::common::{BotAdapter, InlineButton, Markup, MessageContent, MessageId, TargetId};
use crate::core::singbox::hysteria2::Hysteria2ObfsType;
use crate::core::singbox::{SingBoxConfigManager, SingBoxInstaller};
use crate::core::system::SystemMonitor;
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;

/// Send SingBox batch creation results through the adapter (supports routing):
/// header message, chunked link messages, summary message,
/// then best-effort auto-delete after 60 seconds.
pub async fn send_singbox_batch_result(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    protocol_name: &str,
    result: &BatchCreationResult,
    note: Option<&str>,
) -> anyhow::Result<()> {
    let mut message_ids: Vec<String> = Vec::new();

    let mut header_msg = format!(
        "✅ <b>{} 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
        protocol_name,
        result.created_count,
        result.config_file.as_deref().unwrap_or("未知")
    );
    if let Some(note) = note {
        header_msg.push_str(note);
        header_msg.push_str("\n\n");
    }
    if let Ok(msg) = adapter
        .send_message(
            target,
            MessageContent {
                text: header_msg,
                markup: None,
            },
        )
        .await
    {
        message_ids.push(msg.0);
    }

    let mut combined_links = String::new();
    for link in &result.links {
        combined_links.push_str(link);
        combined_links.push_str("\n\n");
    }
    if !combined_links.is_empty()
        && let Ok(msg) = adapter
            .send_message(
                target,
                MessageContent {
                    text: combined_links,
                    markup: None,
                },
            )
            .await
    {
        message_ids.push(msg.0);
    }

    let result_msg = format!("✅ 批量创建完成！\n\n📊 生成数量: {}", result.created_count);
    if let Ok(msg) = adapter
        .send_message(
            target,
            MessageContent {
                text: result_msg,
                markup: None,
            },
        )
        .await
    {
        message_ids.push(msg.0);
    }

    let adapter_clone = adapter.clone();
    let target_clone = target.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(60)).await;
        for id_str in message_ids {
            let mid = MessageId(id_str);
            if let Err(e) = adapter_clone.delete_message(&target_clone, &mid).await {
                log::warn!("删除批量创建消息失败: {}", e);
            }
        }
    });

    Ok(())
}

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();

    match data {
        "m_singbox_mgmt" => {
            let is_installed = SingBoxInstaller::is_installed().await;
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let mut rows = Vec::new();

            if !is_installed {
                rows.push(vec![InlineButton {
                    text: t!("menu.singbox_install").into(),
                    data: "sb_install".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!("menu.singbox_not_installed").into_owned(),
                            markup: Some(Markup { buttons: rows }),
                        },
                    )
                    .await?;
            } else if inbounds.is_empty() {
                rows.push(vec![
                    InlineButton {
                        text: t!("menu.singbox_h2_batch").into(),
                        data: "sb_h2_init".into(),
                    },
                    InlineButton {
                        text: t!("menu.singbox_tu_batch").into(),
                        data: "sb_tu_init".into(),
                    },
                ]);
                rows.push(vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "m_usr".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!("menu.singbox_no_config").into_owned(),
                            markup: Some(Markup { buttons: rows }),
                        },
                    )
                    .await?;
            } else {
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    rows.push(vec![InlineButton {
                        text: format!("📁 {}", filename),
                        data: format!("sb_l:{}", i),
                    }]);
                }
                rows.push(vec![InlineButton {
                    text: t!("menu.singbox_delete_mgmt").into(),
                    data: "sb_del_cfg".into(),
                }]);
                rows.push(vec![
                    InlineButton {
                        text: t!("menu.singbox_h2_batch").into(),
                        data: "sb_h2_init".into(),
                    },
                    InlineButton {
                        text: t!("menu.singbox_tu_batch").into(),
                        data: "sb_tu_init".into(),
                    },
                ]);
                rows.push(vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "m_usr".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!("menu.singbox_mgmt_select").into_owned(),
                            markup: Some(Markup { buttons: rows }),
                        },
                    )
                    .await?;
            }

            Ok(HandlerAction::Done)
        }

        "sb_install" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_installing").into_owned()),
                )
                .await?;

            let adapter = event.adapter.clone();
            let target = event.target.clone();
            tokio::spawn(async move {
                match SingBoxInstaller::install().await {
                    Ok(_) => {
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("menu.singbox_install_success").into_owned(),
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
                                    text: t!(
                                        "menu.singbox_install_fail",
                                        "0" => e.to_string()
                                    )
                                    .into_owned(),
                                    markup: None,
                                },
                            )
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        "sb_h2_init" => {
            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            let mut rows = vec![vec![InlineButton {
                text: "🌐 IPv4".into(),
                data: "sb_h2_ip:4".into(),
            }]];
            if has_ipv6 {
                rows[0].push(InlineButton {
                    text: "🌐 IPv6".into(),
                    data: "sb_h2_ip:6".into(),
                });
            }
            rows.push(vec![InlineButton {
                text: t!("menu.back_user").into(),
                data: "m_singbox_mgmt".into(),
            }]);

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!(
                            "{}\n\n{}",
                            t!("menu.singbox_h2_batch_title"),
                            t!("menu.singbox_h2_batch_ip")
                        ),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_tu_init" => {
            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            let mut rows = vec![vec![InlineButton {
                text: "🌐 IPv4".into(),
                data: "sb_tu_ip:4".into(),
            }]];
            if has_ipv6 {
                rows[0].push(InlineButton {
                    text: "🌐 IPv6".into(),
                    data: "sb_tu_ip:6".into(),
                });
            }
            rows.push(vec![InlineButton {
                text: t!("menu.back_user").into(),
                data: "m_singbox_mgmt".into(),
            }]);

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!(
                            "{}\n\n{}",
                            t!("menu.singbox_tu_batch_title"),
                            t!("menu.singbox_tu_batch_ip")
                        ),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_ip:") => {
            let ip_ver = d.strip_prefix("sb_h2_ip:").unwrap_or("4");
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
            let rows = vec![
                vec![
                    InlineButton {
                        text: "1".into(),
                        data: format!("sb_h2_obfs:{}:1", ip_ver),
                    },
                    InlineButton {
                        text: "3".into(),
                        data: format!("sb_h2_obfs:{}:3", ip_ver),
                    },
                    InlineButton {
                        text: "5".into(),
                        data: format!("sb_h2_obfs:{}:5", ip_ver),
                    },
                ],
                vec![
                    InlineButton {
                        text: "10".into(),
                        data: format!("sb_h2_obfs:{}:10", ip_ver),
                    },
                    InlineButton {
                        text: "20".into(),
                        data: format!("sb_h2_obfs:{}:20", ip_ver),
                    },
                    InlineButton {
                        text: "50".into(),
                        data: format!("sb_h2_obfs:{}:50", ip_ver),
                    },
                ],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "sb_h2_init".into(),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_h2_qty_title", "0" => ip_display).into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_obfs:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_obfs:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };

            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_enable").into(),
                    data: format!("sb_h2_obfs_type:{}:{}", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_disable").into(),
                    data: format!("sb_h2_hop:{}:{}:0", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "sb_h2_init".into(),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "menu.singbox_h2_obfs_title",
                            "0" => ip_display,
                            "1" => count
                        )
                        .into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_obfs_type:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_obfs_type:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };

            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_type_salamander").into(),
                    data: format!("sb_h2_hop:{}:{}:1", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_type_gecko").into(),
                    data: format!("sb_h2_hop:{}:{}:2", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: format!("sb_h2_obfs:{}:{}", ip_ver, count),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "menu.singbox_h2_obfs_type_title",
                            "0" => ip_display,
                            "1" => count
                        )
                        .into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_hop:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_hop:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 3 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let obfs_enabled = parts[2];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
            let obfs_status = match obfs_enabled {
                "2" => t!("menu.singbox_h2_obfs_gecko").to_string(),
                "1" => t!("menu.singbox_h2_obfs_salamander").to_string(),
                _ => t!("menu.singbox_h2_obfs_disabled").to_string(),
            };
            let title = format!(
                "⚡ {} | {} {}\n\n{}",
                ip_display,
                t!("menu.singbox_h2_qty", "0" => count),
                obfs_status,
                t!("menu.singbox_h2_hop_title"),
            );

            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_h2_hop_enable").into(),
                    data: format!("sb_h2_exec:{}:{}:{}:1", ip_ver, count, obfs_enabled),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_h2_hop_disable").into(),
                    data: format!("sb_h2_exec:{}:{}:{}:0", ip_ver, count, obfs_enabled),
                }],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: format!("sb_h2_obfs:{}:{}", ip_ver, count),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: title,
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tu_ip:") => {
            let ip_ver = d.strip_prefix("sb_tu_ip:").unwrap_or("4");
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };
            let rows = vec![
                vec![
                    InlineButton {
                        text: "1".into(),
                        data: format!("sb_tu_exec:{}:1", ip_ver),
                    },
                    InlineButton {
                        text: "3".into(),
                        data: format!("sb_tu_exec:{}:3", ip_ver),
                    },
                    InlineButton {
                        text: "5".into(),
                        data: format!("sb_tu_exec:{}:5", ip_ver),
                    },
                ],
                vec![
                    InlineButton {
                        text: "10".into(),
                        data: format!("sb_tu_exec:{}:10", ip_ver),
                    },
                    InlineButton {
                        text: "20".into(),
                        data: format!("sb_tu_exec:{}:20", ip_ver),
                    },
                    InlineButton {
                        text: "50".into(),
                        data: format!("sb_tu_exec:{}:50", ip_ver),
                    },
                ],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "sb_tu_init".into(),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_tu_qty_title", "0" => ip_display).into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_h2_exec:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_exec:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 4 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count: usize = parts[1].parse().unwrap_or(1);
            let obfs_type = match parts[2] {
                "1" => Some(Hysteria2ObfsType::Salamander),
                "2" => Some(Hysteria2ObfsType::Gecko),
                _ => None,
            };
            let hopping_enabled: bool = parts[3] == "1";
            let ip_version = if ip_ver == "6" {
                IpVersion::IPv6
            } else {
                IpVersion::IPv4
            };

            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_creating").into_owned()),
                )
                .await?;

            let adapter = event.adapter.clone();
            let target = event.target.clone();
            let is_gecko = matches!(obfs_type, Some(Hysteria2ObfsType::Gecko));

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_hysteria2(
                    count,
                    ip_version,
                    obfs_type,
                    hopping_enabled,
                )
                .await
                {
                    Ok(result) => {
                        let note = if is_gecko {
                            Some(t!("menu.singbox_h2_gecko_note").to_string())
                        } else {
                            None
                        };
                        if let Err(e) = send_singbox_batch_result(
                            adapter.clone(),
                            &target,
                            "Hysteria2",
                            &result,
                            note.as_deref(),
                        )
                        .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
                    Err(e) => {
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("menu.singbox_create_fail", "0" => e.to_string())
                                        .into_owned(),
                                    markup: None,
                                },
                            )
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_tu_exec:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_tu_exec:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count: usize = parts[1].parse().unwrap_or(1);
            let ip_version = if ip_ver == "6" {
                IpVersion::IPv6
            } else {
                IpVersion::IPv4
            };

            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_creating").into_owned()),
                )
                .await?;

            let adapter = event.adapter.clone();
            let target = event.target.clone();

            tokio::spawn(async move {
                match SingBoxConfigManager::batch_create_tuic(count, ip_version).await {
                    Ok(result) => {
                        if let Err(e) =
                            send_singbox_batch_result(
                                adapter.clone(),
                                &target,
                                "TUIC",
                                &result,
                                None,
                            )
                                .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
                    Err(e) => {
                        let _ = adapter
                            .send_message(
                                &target,
                                MessageContent {
                                    text: t!("menu.singbox_create_fail", "0" => e.to_string())
                                        .into_owned(),
                                    markup: None,
                                },
                            )
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        "sb_del_cfg" => {
            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_del_all").into(),
                    data: "sb_del_all_confirm".into(),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_del_count").into(),
                    data: "sb_del_count".into(),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_del_select").into(),
                    data: "sb_del_select".into(),
                }],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "m_singbox_mgmt".into(),
                }],
            ];
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_del_title").into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_del_all_confirm" => {
            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_confirm_clear").into(),
                    data: "sb_del_all_exec".into(),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_cancel").into(),
                    data: "sb_del_cfg".into(),
                }],
            ];
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_confirm_del_all").into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        "sb_del_all_exec" => {
            match SingBoxConfigManager::delete_all_configurations().await {
                Ok(count) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(
                                t!(
                                    "menu.singbox_del_success_all",
                                    "0" => count.to_string()
                                )
                                .into_owned(),
                            ),
                        )
                        .await?;
                }
                Err(e) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("menu.singbox_del_fail", "0" => e.to_string()).into_owned()),
                        )
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("sb_del_cfg".to_string()))
        }

        "sb_del_count" => {
            let rows = vec![
                vec![
                    InlineButton {
                        text: t!("menu.singbox_count_10").into(),
                        data: "sb_del_exec_count:10".into(),
                    },
                    InlineButton {
                        text: t!("menu.singbox_count_50").into(),
                        data: "sb_del_exec_count:50".into(),
                    },
                ],
                vec![
                    InlineButton {
                        text: t!("menu.singbox_count_100").into(),
                        data: "sb_del_exec_count:100".into(),
                    },
                    InlineButton {
                        text: t!("menu.singbox_count_500").into(),
                        data: "sb_del_exec_count:500".into(),
                    },
                ],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "sb_del_cfg".into(),
                }],
            ];
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_del_count_title").into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_del_exec_count:") => {
            let n: usize = d
                .strip_prefix("sb_del_exec_count:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            match SingBoxConfigManager::delete_by_count(n).await {
                Ok(deleted) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(
                                t!(
                                    "menu.singbox_del_success_count",
                                    "0" => deleted.to_string()
                                )
                                .into_owned(),
                            ),
                        )
                        .await?;
                }
                Err(e) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("menu.singbox_del_fail", "0" => e.to_string()).into_owned()),
                        )
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("sb_del_cfg".to_string()))
        }

        "sb_del_select" => {
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let count = SingBoxConfigManager::get_config_count().await.unwrap_or(0);

            if inbounds.is_empty() {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_no_files").into_owned()),
                    )
                    .await?;
            } else {
                let mut rows = Vec::new();
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    rows.push(vec![InlineButton {
                        text: format!("🗑️ {}", filename),
                        data: format!("sb_del_file:{}", i),
                    }]);
                }
                rows.push(vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: "sb_del_cfg".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!(
                                "menu.singbox_del_select_title",
                                "0" => count.to_string()
                            )
                            .into_owned(),
                            markup: Some(Markup { buttons: rows }),
                        },
                    )
                    .await?;
            }

            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_del_file:") => {
            let index: usize = d
                .strip_prefix("sb_del_file:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();

            if let Some(path) = inbounds.get(index) {
                match SingBoxConfigManager::delete_specific_configuration(path).await {
                    Ok(()) => {
                        let filename = path.split('/').next_back().unwrap_or("Unknown");
                        event
                            .adapter
                            .answer_callback(
                                &event.target,
                                &event.callback_id,
                                Some(
                                    t!(
                                        "menu.singbox_del_success_specific",
                                        "0" => filename
                                    )
                                    .into_owned(),
                                ),
                            )
                            .await?;
                    }
                    Err(e) => {
                        event
                            .adapter
                            .answer_callback(
                                &event.target,
                                &event.callback_id,
                                Some(
                                    t!("menu.singbox_del_fail", "0" => e.to_string()).into_owned(),
                                ),
                            )
                            .await?;
                    }
                }
            } else {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_invalid_index").into_owned()),
                    )
                    .await?;
            }
            Ok(HandlerAction::Redirect("sb_del_select".to_string()))
        }

        _ => Ok(HandlerAction::Done),
    }
}
