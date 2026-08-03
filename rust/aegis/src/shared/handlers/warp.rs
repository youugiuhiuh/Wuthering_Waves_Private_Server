use crate::common::{InlineButton, Markup, MessageContent};
use crate::core::xray::installer::WarpInstaller;
use crate::core::xray::{ConfigManager, WarpMode};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::utils;
use rust_i18n::t;
use sha2::{Digest, Sha256};

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();

    match data {
        "m_warp" => {
            let is_installed = WarpInstaller::is_installed().await;
            if !is_installed {
                let markup = Markup {
                    buttons: vec![
                        vec![InlineButton {
                            text: t!("warp.install_warp").into(),
                            data: "a_inst_warp".into(),
                        }],
                        vec![InlineButton {
                            text: t!("menu.back_net_opt").into(),
                            data: "m_net_opt".into(),
                        }],
                    ],
                };
                event
                    .output
                    .as_adapter()
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!("warp.not_installed").into(),
                            markup: Some(markup),
                        },
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));

            let rule_display = if current_rules.is_empty() {
                t!("warp.no_rules").to_string()
            } else {
                let escaped_rules: Vec<String> = current_rules
                    .iter()
                    .map(|r| utils::escape_html(r))
                    .collect();
                if escaped_rules.len() > 5 {
                    format!(
                        "{} ({} {})",
                        escaped_rules
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", "),
                        t!("warp.total_count_prefix"),
                        escaped_rules.len()
                    )
                } else {
                    escaped_rules.join(", ")
                }
            };

            let markup = Markup {
                buttons: vec![
                    vec![
                        InlineButton {
                            text: t!("warp.add_rule").into(),
                            data: "a_warp_add_input".into(),
                        },
                        InlineButton {
                            text: t!("warp.del_rule").into(),
                            data: "a_warp_del_menu".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("warp.mode_label", "0" => current_mode.as_str()).into(),
                        data: "a_warp_switch_mode".into(),
                    }],
                    vec![InlineButton {
                        text: t!("warp.status_check").into(),
                        data: "a_warp_status".into(),
                    }],
                    vec![
                        InlineButton {
                            text: t!("warp.restart_service").into(),
                            data: "a_warp_restart".into(),
                        },
                        InlineButton {
                            text: t!("warp.uninstall_service").into(),
                            data: "a_warp_uninstall".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("warp.clear_all").into(),
                        data: "a_warp_clear_confirm".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.back_net_opt").into(),
                        data: "m_net_opt".into(),
                    }],
                ],
            };

            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "warp.title",
                            "0" => current_mode.as_str(),
                            "1" => rule_display
                        )
                        .into(),
                        markup: Some(markup),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_switch_mode" => {
            let (current_rules, current_mode) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));
            let next_mode = current_mode.next();

            match ConfigManager::update_warp_routing_rules(current_rules, next_mode).await {
                Ok(_) => Ok(HandlerAction::Redirect("m_warp".to_string())),
                Err(e) => {
                    event
                        .output
                        .as_adapter()
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.mode_switch_fail", "0" => e.to_string()).into()),
                        )
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_inst_warp" => {
            event
                .output
                .as_adapter()
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.installing").into()),
                )
                .await?;
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.installing").into(),
                        markup: None,
                    },
                )
                .await?;

            match WarpInstaller::install().await {
                Ok(_) => {
                    event
                        .output
                        .as_adapter()
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!("warp.install_success").into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    event
                        .output
                        .as_adapter()
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!("warp.install_fail", "0" => e.to_string()).into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_warp_add_input" => {
            event
                .output
                .as_adapter()
                .send_message(
                    &event.target,
                    MessageContent {
                        text: t!("warp.add_input").into(),
                        markup: None,
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_del_menu" => {
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or((Vec::new(), WarpMode::Default));

            if current_rules.is_empty() {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("warp.no_rules_del").into()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }

            let mut buttons = Vec::new();
            for rule in current_rules.iter() {
                let mut hasher = Sha256::new();
                hasher.update(rule.as_bytes());
                let hash = hex::encode(hasher.finalize());
                let short_hash = &hash[..8];

                let display_rule = if rule.len() > 30 {
                    format!("{}...", utils::escape_html(&rule[..27]))
                } else {
                    utils::escape_html(rule)
                };

                buttons.push(vec![InlineButton {
                    text: format!("\u{1F5D1} {}", display_rule),
                    data: format!("a_warp_del:{}", short_hash),
                }]);
            }
            buttons.push(vec![InlineButton {
                text: t!("menu.back").into(),
                data: "m_warp".into(),
            }]);

            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.del_title").into(),
                        markup: Some(Markup { buttons }),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        d if d.starts_with("a_warp_del:") => {
            let hash_prefix = d.strip_prefix("a_warp_del:").unwrap_or("");
            if let Err(e) = utils::validate_hash_prefix(hash_prefix) {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(format!("\u{274C} {}", e)),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or_default();

            let rule_to_delete = current_rules.iter().find(|r| {
                let mut hasher = Sha256::new();
                hasher.update(r.as_bytes());
                let hash = hex::encode(hasher.finalize());
                &hash[..8] == hash_prefix
            });

            if let Some(rule) = rule_to_delete {
                let markup = Markup {
                    buttons: vec![
                        vec![InlineButton {
                            text: t!("warp.confirm_del").into(),
                            data: format!("a_warp_del_confirm:{}", hash_prefix),
                        }],
                        vec![InlineButton {
                            text: t!("warp.cancel_del").into(),
                            data: "a_warp_del_menu".into(),
                        }],
                    ],
                };

                event
                    .output
                    .as_adapter()
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: t!("warp.del_confirm", "0" => utils::escape_html(rule)).into(),
                            markup: Some(markup),
                        },
                    )
                    .await?;
            } else {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("warp.rule_not_found").into()),
                    )
                    .await?;
                return Ok(HandlerAction::Redirect("a_warp_del_menu".to_string()));
            }
            Ok(HandlerAction::Done)
        }
        d if d.starts_with("a_warp_del_confirm:") => {
            let hash_prefix = d.strip_prefix("a_warp_del_confirm:").unwrap_or("");
            if let Err(e) = utils::validate_hash_prefix(hash_prefix) {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(format!("\u{274C} {}", e)),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _) = ConfigManager::get_warp_routing_rules()
                .await
                .unwrap_or_default();

            let rule_to_delete = current_rules.into_iter().find(|r| {
                let mut hasher = Sha256::new();
                hasher.update(r.as_bytes());
                let hash = hex::encode(hasher.finalize());
                &hash[..8] == hash_prefix
            });

            if let Some(rule) = rule_to_delete {
                match ConfigManager::remove_warp_routing_rule(&rule).await {
                    Ok(_) => {
                        event
                            .output
                            .as_adapter()
                            .answer_callback(
                                &event.target,
                                &event.callback_id,
                                Some(t!("warp.rule_deleted").into()),
                            )
                            .await?;
                    }
                    Err(e) => {
                        event
                            .output
                            .as_adapter()
                            .answer_callback(
                                &event.target,
                                &event.callback_id,
                                Some(t!("warp.del_fail", "0" => e.to_string()).into()),
                            )
                            .await?;
                    }
                }
            } else {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("warp.rule_not_found").into()),
                    )
                    .await?;
            }
            Ok(HandlerAction::Redirect("a_warp_del_menu".to_string()))
        }
        "a_warp_clear_confirm" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("warp.confirm_clear").into(),
                        data: "a_warp_clear_exec".into(),
                    }],
                    vec![InlineButton {
                        text: t!("warp.cancel_del").into(),
                        data: "m_warp".into(),
                    }],
                ],
            };
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.clear_confirm").into(),
                        markup: Some(markup),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_clear_exec" => {
            match ConfigManager::update_warp_routing_rules(Vec::new(), WarpMode::Default).await {
                Ok(_) => {
                    event
                        .output
                        .as_adapter()
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.all_cleared").into()),
                        )
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    event
                        .output
                        .as_adapter()
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.clear_fail", "0" => e.to_string()).into()),
                        )
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        "a_warp_status" => match WarpInstaller::status().await {
            Ok(status) => {
                let markup = Markup {
                    buttons: vec![vec![InlineButton {
                        text: t!("menu.back").into(),
                        data: "m_warp".into(),
                    }]],
                };
                event
                    .output
                    .as_adapter()
                    .edit_message(
                        &event.target,
                        &event.msg_id,
                        MessageContent {
                            text: format!(
                                "\u{1F4CA} <b>WARP {}</b>\n\n{}",
                                t!("warp.status_label"),
                                status
                            ),
                            markup: Some(markup),
                        },
                    )
                    .await?;
                Ok(HandlerAction::Done)
            }
            Err(e) => {
                event
                    .output
                    .as_adapter()
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("warp.status_fail", "0" => e.to_string()).into()),
                    )
                    .await?;
                Ok(HandlerAction::Done)
            }
        },
        "a_warp_restart" => {
            event
                .output
                .as_adapter()
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.restarting").into()),
                )
                .await?;
            match WarpInstaller::restart_service().await {
                Ok(_) => {
                    event
                        .output
                        .as_adapter()
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.restart_success").into()),
                        )
                        .await?;
                }
                Err(e) => {
                    event
                        .output
                        .as_adapter()
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!("warp.restart_fail", "0" => e.to_string()).into(),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall" => {
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("warp.confirm_uninstall").into(),
                        data: "a_warp_uninstall_confirm".into(),
                    }],
                    vec![InlineButton {
                        text: t!("warp.cancel_del").into(),
                        data: "m_warp".into(),
                    }],
                ],
            };
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.uninstall_confirm").into(),
                        markup: Some(markup),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_uninstall_confirm" => {
            event
                .output
                .as_adapter()
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.uninstalling").into()),
                )
                .await?;
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.uninstalling").into(),
                        markup: None,
                    },
                )
                .await?;

            match WarpInstaller::uninstall().await {
                Ok(_) => {
                    event
                        .output
                        .as_adapter()
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!("warp.uninstall_success").into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    event
                        .output
                        .as_adapter()
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!("warp.uninstall_fail", "0" => e.to_string()).into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        _ => Ok(HandlerAction::Done),
    }
}
