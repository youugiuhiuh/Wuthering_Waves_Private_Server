use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::paths::{singbox, xray};
use crate::core::singbox::SingBoxInstaller;
use crate::core::system::SystemMonitor;
use crate::core::system::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use rust_i18n::t;
use std::path::Path;

use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};

const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 10 * 60;
const BOT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn send_main_menu(adapter: &dyn BotAdapter, target: &TargetId) -> anyhow::Result<()> {
    let mut rows = vec![
        vec![
            InlineButton {
                text: t!("menu.monitor").into(),
                data: "m_mon".into(),
            },
            InlineButton {
                text: t!("menu.users").into(),
                data: "m_usr".into(),
            },
        ],
        vec![InlineButton {
            text: t!("menu.ops").into(),
            data: "m_ops_center".into(),
        }],
        vec![InlineButton {
            text: t!("menu.settings").into(),
            data: "m_settings".into(),
        }],
    ];
    rows.push(vec![InlineButton {
        text: t!("menu.one_click_deploy").into(),
        data: "a_one_click".into(),
    }]);
    if !crate::core::i18n::is_lang_configured() {
        rows.push(vec![
            InlineButton {
                text: t!("lang.zh").into(),
                data: "lang:zh".into(),
            },
            InlineButton {
                text: t!("lang.en").into(),
                data: "lang:en".into(),
            },
            InlineButton {
                text: t!("lang.ja").into(),
                data: "lang:ja".into(),
            },
        ]);
    }
    let markup = Markup { buttons: rows };
    adapter
        .send_message(
            target,
            MessageContent {
                text: format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
                markup: Some(markup),
            },
        )
        .await?;
    Ok(())
}

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    match data {
        "m_main" => {
            let mut kb_rows = vec![
                vec![
                    InlineButton {
                        text: t!("menu.monitor").into(),
                        data: "m_mon".into(),
                    },
                    InlineButton {
                        text: t!("menu.users").into(),
                        data: "m_usr".into(),
                    },
                ],
                vec![
                    InlineButton {
                        text: t!("menu.ops").into(),
                        data: "m_ops_center".into(),
                    },
                    InlineButton {
                        text: t!("menu.settings").into(),
                        data: "m_settings".into(),
                    },
                ],
            ];
            kb_rows.push(vec![InlineButton {
                text: t!("menu.one_click_deploy").into(),
                data: "a_one_click".into(),
            }]);
            let markup = Markup { buttons: kb_rows };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_ops_center" => {
            let markup = Markup {
                buttons: vec![
                    vec![
                        InlineButton {
                            text: t!("menu.bbr3_install").into(),
                            data: "a_bbr3".into(),
                        },
                        InlineButton {
                            text: t!("menu.generic_tune").into(),
                            data: "a_tune".into(),
                        },
                    ],
                    vec![
                        InlineButton {
                            text: t!("menu.network_opt").into(),
                            data: "m_net_opt".into(),
                        },
                        InlineButton {
                            text: t!("menu.security").into(),
                            data: "m_security".into(),
                        },
                    ],
                    vec![
                        InlineButton {
                            text: t!("menu.sys_cmd").into(),
                            data: "m_sys_cmd".into(),
                        },
                        InlineButton {
                            text: t!("menu.log_audit").into(),
                            data: "m_log".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("menu.back_main").into(),
                        data: "m_main".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.ops_center").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_settings" => {
            let timeout = event.session_timeout_secs;
            let timeout_label = format!(
                "{}",
                t!("menu.session_timeout", "0" => crate::utils::format_duration_human(timeout))
            );
            let markup = Markup {
                buttons: vec![
                    vec![
                        InlineButton {
                            text: t!("menu.wwps_core_btn").into(),
                            data: "a_wwps_core_menu".into(),
                        },
                        InlineButton {
                            text: t!("menu.singbox_mgmt_btn").into(),
                            data: "a_wwps_box_menu".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("schedule.add_task").into(),
                        data: "m_sched".into(),
                    }],
                    vec![
                        InlineButton {
                            text: t!("schedule.geo_update_now").into(),
                            data: "a_geo_menu".into(),
                        },
                        InlineButton {
                            text: t!("ops.bot_self_update").into(),
                            data: "a_upgrade".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: timeout_label.into(),
                        data: "m_session_timeout".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.danger_zone").into(),
                        data: "m_danger".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_main").into(),
                        data: "m_main".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.settings_desc").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_net_opt" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("menu.warp_btn").into(),
                        data: "m_warp".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_ops").into(),
                        data: "m_ops_center".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!(
                            "🌩 <b>{}</b>\n{}",
                            t!("menu.network_opt"),
                            t!("menu.network_opt_desc")
                        ),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_security" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("menu.security_button").into(),
                        data: "a_fw".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_ops").into(),
                        data: "m_ops_center".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.security_desc").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_sys_cmd" => {
            let markup = Markup {
                buttons: vec![
                    vec![
                        InlineButton {
                            text: t!("ops.sys_update_btn").into(),
                            data: "a_sys_update".into(),
                        },
                        InlineButton {
                            text: t!("ops.sys_reboot_btn").into(),
                            data: "a_sys_reboot".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("menu.back_ops").into(),
                        data: "m_ops_center".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.sys_cmd_desc").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "a_geo_menu" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("schedule.geo_update_now").into(),
                        data: "a_geo".into(),
                    }],
                    vec![InlineButton {
                        text: t!("schedule.geo_auto_sched").into(),
                        data: "a_geo_sched_menu".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("schedule.geo_scheduled_title").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_mon" => {
            let report = SystemMonitor::get_status_report()
                .await
                .unwrap_or_else(|e| t!("ops.monitor_fail", "0" => e).into_owned());
            let (wwps_core, wwps_box) = SystemMonitor::get_core_status().await;

            let status_text = format!(
                "{}\n\n🤖 <b>{}</b>: v{}\n\n⚙️ <b>{}</b>:\n- Xray-core: {}\n- Sing-box: {}",
                report,
                t!("menu.monitor"),
                BOT_VERSION,
                t!("menu.settings"),
                if wwps_core { "🟢" } else { "🔴" },
                if wwps_box { "🟢" } else { "🔴" }
            );

            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("menu.refresh").into(),
                        data: "m_mon".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back").into(),
                        data: "m_main".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: status_text,
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "m_usr" => {
            let wwps_core_config_exists = Path::new(xray::CONF_DIR).exists();
            let singbox_config_exists = Path::new(singbox::CONF_DIR).exists();
            let mut buttons = Vec::new();

            if !wwps_core_config_exists && !singbox_config_exists {
                buttons.push(vec![InlineButton {
                    text: t!("menu.init_reality_base").into(),
                    data: "a_inst_base".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: format!(
                                "{}\n\n❌ <b>{}</b>\n\n{}",
                                t!("menu.users"),
                                t!("menu.no_proxy_core"),
                                t!("menu.settings_desc")
                            ),
                            markup: Some(Markup { buttons }),
                        },
                    )
                    .await?;
            } else {
                buttons.push(vec![InlineButton {
                    text: t!("menu.wwps_core_btn").into(),
                    data: "m_xray_mgmt".into(),
                }]);
                buttons.push(vec![InlineButton {
                    text: t!("menu.singbox_mgmt_btn").into(),
                    data: "m_singbox_mgmt".into(),
                }]);
                buttons.push(vec![InlineButton {
                    text: t!("menu.back").into(),
                    data: "m_main".into(),
                }]);
                event
                    .adapter
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: format!("{}\n\n{}", t!("menu.users"), t!("menu.settings_desc")),
                            markup: Some(Markup { buttons }),
                        },
                    )
                    .await?;
            }
        }
        "m_session_timeout" => {
            let current = event.session_timeout_secs;
            let options: Vec<(u64, String)> = vec![
                (5 * 60, crate::utils::format_duration_human(5 * 60)),
                (10 * 60, crate::utils::format_duration_human(10 * 60)),
                (30 * 60, crate::utils::format_duration_human(30 * 60)),
                (60 * 60, crate::utils::format_duration_human(60 * 60)),
                (4 * 3600, crate::utils::format_duration_human(4 * 3600)),
                (12 * 3600, crate::utils::format_duration_human(12 * 3600)),
                (24 * 3600, crate::utils::format_duration_human(24 * 3600)),
            ];
            let mut rows = Vec::new();
            for chunk in options.chunks(3) {
                let row: Vec<InlineButton> = chunk
                    .iter()
                    .map(|(secs, label)| {
                        let prefix = if *secs == current { "✅ " } else { "" };
                        InlineButton {
                            text: format!("{}{}", prefix, label),
                            data: format!("set_timeout:{}", secs),
                        }
                    })
                    .collect();
                rows.push(row);
            }
            rows.push(vec![InlineButton {
                text: t!("menu.back_settings").into(),
                data: "m_settings".into(),
            }]);

            event.adapter.edit_message(&event.target, &event.msg_id, MessageContent {
                text: format!(
                    "{}\n\n<b>{}</b>: {}\n\n{}",
                    t!("menu.session_timeout", "0" => crate::utils::format_duration_human(current)),
                    t!("menu.session_timeout"),
                    crate::utils::format_duration_human(current),
                    t!("menu.session_timeout_desc")
                ),
                markup: Some(Markup { buttons: rows }),
            }).await?;
        }
        d if d.starts_with("set_timeout:") => {
            let secs: u64 = d
                .strip_prefix("set_timeout:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS);

            event.adapter.answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("callback.session_timeout_set", "0" => crate::utils::format_duration_human(secs)).into_owned()),
            ).await?;

            return Ok(HandlerAction::Redirect("m_session_timeout".to_string()));
        }
        "m_danger" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("destruct.destroy_btn").into(),
                        data: "a_destroy_ask".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!(
                            "{}\n\n{}",
                            t!("menu.danger_zone"),
                            t!("menu.danger_zone_desc")
                        ),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "a_wwps_core_menu" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("schedule.geo_update_now").into(),
                        data: "a_wwps_core_latest".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.version_tags").into(),
                        data: "a_wwps_core_tags".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.wwps_core_mgmt").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "a_wwps_core_latest" => {
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
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(None, adapter.as_ref(), &target).await
                {
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
            });
        }
        "a_wwps_core_tags" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.version_tags").into_owned()),
                )
                .await?;

            let reply = match WwpsCoreUpgradeConfig::from_env()
                .and_then(WwpsCoreUpgradeManager::new)
            {
                Ok(manager) => match manager.fetch_recent_tags(5).await {
                    Ok(tags) if !tags.is_empty() => {
                        let mut buttons = Vec::new();
                        for tag in tags {
                            buttons.push(vec![InlineButton {
                                text: format!("⬆️ {}", tag),
                                data: format!("wwps_core_tag:{}", tag),
                            }]);
                        }
                        buttons.push(vec![InlineButton {
                            text: t!("menu.back_settings").into(),
                            data: "a_wwps_core_menu".into(),
                        }]);
                        event
                            .adapter
                            .edit_message(
                                &event.target,
                                &event.msg_id,
                                MessageContent {
                                    text: t!("menu.wwps_core_mgmt").into_owned(),
                                    markup: Some(Markup { buttons }),
                                },
                            )
                            .await
                    }
                    Ok(_) => {
                        event
                            .adapter
                            .edit_message(
                                &event.target,
                                &event.msg_id,
                                MessageContent {
                                    text: t!("menu.no_version_found").into_owned(),
                                    markup: None,
                                },
                            )
                            .await
                    }
                    Err(err) => {
                        event
                            .adapter
                            .edit_message(
                                &event.target,
                                &event.msg_id,
                                MessageContent {
                                    text: t!("ops.upgrade_fail", "0" => err.to_string())
                                        .into_owned(),
                                    markup: None,
                                },
                            )
                            .await
                    }
                },
                Err(err) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("ops.upgrade_fail", "0" => err.to_string()).into_owned(),
                                markup: None,
                            },
                        )
                        .await
                }
            };

            if reply.is_err() {
                let _ = event
                    .adapter
                    .send_message(
                        &event.target,
                        MessageContent {
                            text: t!("ops.upgrade_fail", "0" => "").into_owned(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
        d if d.starts_with("wwps_core_tag:") => {
            let tag = d.strip_prefix("wwps_core_tag:").unwrap_or("").to_string();
            if tag.is_empty() {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.version_tag_empty").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }

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
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(Some(tag), adapter.as_ref(), &target).await
                {
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
            });
        }
        "a_wwps_box_menu" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("ops.singbox_restart").into(),
                        data: "a_wwps_box_restart".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.singbox_status").into(),
                        data: "a_wwps_box_status".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_settings").into(),
                        data: "m_settings".into(),
                    }],
                ],
            };

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("menu.singbox_mgmt_title").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "a_wwps_box_restart" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("ops.singbox_restart").into_owned()),
                )
                .await?;

            match SingBoxInstaller::restart_service().await {
                Ok(_) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("ops.singbox_restart_success").into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
                Err(err) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("ops.singbox_restart_fail", "0" => err.to_string())
                                    .into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
        }
        "a_wwps_box_status" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("menu.singbox_status").into_owned()),
                )
                .await?;

            match SingBoxInstaller::status().await {
                Ok(status) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: format!("{}\n\n{}", t!("menu.singbox_mgmt_title"), status),
                                markup: None,
                            },
                        )
                        .await?;
                }
                Err(err) => {
                    event
                        .adapter
                        .edit_message(
                            &event.target,
                            &event.msg_id,
                            MessageContent {
                                text: t!("ops.singbox_status_fail", "0" => err.to_string())
                                    .into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
        }
        _ => {}
    }
    Ok(HandlerAction::Done)
}
