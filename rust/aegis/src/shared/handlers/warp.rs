use crate::common::{InlineButton, Markup, MessageContent};
use crate::core::xray::installer::WarpInstaller;
use crate::core::xray::{ConfigManager, WarpMode, WarpScope};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::utils;
use rust_i18n::t;
use sha2::{Digest, Sha256};

/// 删除菜单里按钮显示的规则名上限（**字符**数，不是字节数）。
const BUTTON_RULE_MAX_CHARS: usize = 27;
const BUTTON_RULE_MAX_BYTES: usize = 30;

/// 把规则截短到适合按钮显示的宽度，并转义 HTML。
///
/// 修复前直接写 `&rule[..27]`——字节切片。规则名来自管理员自由输入
/// （`message.rs` 对该输入**没有任何校验**），一旦第 27 字节落在一个
/// 多字节字符内部（中文注释、IDN 域名等），这里会 panic:
///
/// ```text
/// byte index 27 is not a char boundary; it is inside '中'
/// ```
///
/// 而这条路径在每个删除菜单渲染时都会跑到，所以一个含中文的长规则
/// 会让「删除规则」按钮永久崩溃。改用 `chars()` 按字符计数。
fn truncate_for_button(rule: &str) -> String {
    let escaped = utils::escape_html(rule);
    if rule.len() <= BUTTON_RULE_MAX_BYTES {
        return escaped;
    }
    // 按**字符**取前缀，不可能切在多字节字符中间。
    let head: String = rule.chars().take(BUTTON_RULE_MAX_CHARS).collect();
    format!("{}...", utils::escape_html(&head))
}

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();

    match data {
        "m_warp" => {
            // 一次判定，两处使用：既是「未安装」拦截的判据，也是
            // 界面显示「装的是哪一套」的依据。
            let installed_kinds = crate::core::usque::installer::detect_installed().await;
            let is_installed = installed_kinds.any();
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
                    .adapter
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

            let (current_rules, current_mode, current_scope) =
                ConfigManager::get_warp_routing_rules().await.unwrap_or((
                    Vec::new(),
                    WarpMode::Default,
                    WarpScope::default(),
                ));

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
                    // 流量范围：二态互斥切换。
                    // v1 不生成 catch-all 规则，故文案必须显式标注未生效，
                    // 否则管理员会以为点一下就已经「全部流量走 WARP」了。
                    vec![InlineButton {
                        text: t!(
                            "warp.scope_button",
                            "0" => t!(current_scope.i18n_key())
                        )
                        .into(),
                        data: "a_warp_switch_scope".into(),
                    }],
                    vec![InlineButton {
                        text: t!("warp.status_check").into(),
                        data: "a_warp_status".into(),
                    }],
                    // 出口连通性探测：分别查 UDP 与 TCP/443 两条路。
                    vec![InlineButton {
                        text: t!("warp.probe_button").into(),
                        data: "a_warp_probe".into(),
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
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "warp.title",
                            "0" => current_mode.as_str(),
                            "1" => rule_display,
                            // 当前装的是哪一套。两套并存且不可互换（内核 TUN
                            // vs 本地 SOCKS），笼统地说「WARP 已安装」会让
                            // 管理员误以为走的是另一条路。
                            "2" => t!(WarpInstaller::installed_banner_key(installed_kinds))
                                .to_string()
                        )
                        .into(),
                        markup: Some(markup),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_warp_switch_mode" => {
            let (current_rules, current_mode, current_scope) =
                ConfigManager::get_warp_routing_rules().await.unwrap_or((
                    Vec::new(),
                    WarpMode::Default,
                    WarpScope::default(),
                ));
            let next_mode = current_mode.next();

            // 改出口协议需要重写路由文件，故 scope_only = false。
            match ConfigManager::update_warp_routing_rules(
                current_rules,
                next_mode,
                current_scope,
                false,
            )
            .await
            {
                Ok(_) => Ok(HandlerAction::Redirect("m_warp".to_string())),
                Err(e) => {
                    event
                        .adapter
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
        // 流量范围二态互斥切换。
        //
        // `scope_only = true`：只写 `warp_account.json`，**不**重写路由规则、
        // **不**重启 `wwps-core`。v1 的 Global 不改变任何路由行为，为一个纯状态
        // 变更去重启核心只会白白抖动全体在线用户的连接。
        "a_warp_switch_scope" => {
            let (current_rules, current_mode, current_scope) =
                ConfigManager::get_warp_routing_rules().await.unwrap_or((
                    Vec::new(),
                    WarpMode::Default,
                    WarpScope::default(),
                ));
            let next_scope = current_scope.next();

            match ConfigManager::update_warp_routing_rules(
                current_rules,
                current_mode,
                next_scope,
                true,
            )
            .await
            {
                Ok(_) => Ok(HandlerAction::Redirect("m_warp".to_string())),
                Err(e) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.scope_switch_fail", "0" => e.to_string()).into()),
                        )
                        .await?;
                    Ok(HandlerAction::Done)
                }
            }
        }
        // 出口连通性探测：并行跑 UDP 与 TCP 两路，给出组合结论。
        "a_warp_probe" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.probe_running").into()),
                )
                .await?;

            let report = crate::core::network::warp_probe::run_probe().await;

            let udp_text = match report.udp {
                crate::core::network::UdpVerdict::Available => t!("warp.probe_yes"),
                crate::core::network::UdpVerdict::Blocked => t!("warp.probe_no"),
                crate::core::network::UdpVerdict::ProbeFailed => t!("warp.probe_unknown"),
            };
            let tcp_text = match report.tcp {
                crate::core::network::TcpVerdict::AvailableH2 => t!("warp.probe_yes"),
                crate::core::network::TcpVerdict::OpenNoH2 => t!("warp.probe_unknown"),
                crate::core::network::TcpVerdict::Blocked => t!("warp.probe_no"),
            };
            let conclusion = match report.conclusion {
                crate::core::network::ProbeConclusion::WgUsable => t!("warp.probe_conclusion_wg"),
                crate::core::network::ProbeConclusion::TcpOnly => t!("warp.probe_conclusion_tcp"),
                crate::core::network::ProbeConclusion::UdpOnly => t!("warp.probe_conclusion_udp"),
                crate::core::network::ProbeConclusion::FullyBlocked => {
                    t!("warp.probe_conclusion_blocked")
                }
            };

            let markup = Markup {
                buttons: vec![vec![InlineButton {
                    text: t!("menu.back").into(),
                    data: "m_warp".into(),
                }]],
            };

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!(
                            "{}\n\n{}: {}\n{}: {}\n\n{}",
                            t!("warp.probe_title"),
                            t!("warp.probe_udp_label"),
                            udp_text,
                            t!("warp.probe_tcp_label"),
                            tcp_text,
                            conclusion
                        ),
                        markup: Some(markup),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }
        "a_inst_warp" => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.installing").into()),
                )
                .await?;
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.installing").into(),
                        markup: None,
                    },
                )
                .await?;

            // 按探测结果只装需要的那一套（决策 W2）。
            //
            // `install_smart` 内部：实时探测 → 选实现 → 安装 → 写探测缓存。
            // 写缓存这一步很关键——否则 current_egress() 永远读不到结论，
            // 装好了也不会启用。
            let arch = std::env::consts::ARCH;
            match WarpInstaller::install_smart(arch).await {
                Ok(plan) => {
                    event
                        .adapter
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!(plan.i18n_key()).into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    event
                        .adapter
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
                .adapter
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
            let (current_rules, _, _) = ConfigManager::get_warp_routing_rules().await.unwrap_or((
                Vec::new(),
                WarpMode::Default,
                WarpScope::default(),
            ));

            if current_rules.is_empty() {
                event
                    .adapter
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

                let display_rule = truncate_for_button(rule);

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
                .adapter
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
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(format!("\u{274C} {}", e)),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _, _) = ConfigManager::get_warp_routing_rules()
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
                    .adapter
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
                    .adapter
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
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(format!("\u{274C} {}", e)),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let (current_rules, _, _) = ConfigManager::get_warp_routing_rules()
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
                            .adapter
                            .answer_callback(
                                &event.target,
                                &event.callback_id,
                                Some(t!("warp.rule_deleted").into()),
                            )
                            .await?;
                    }
                    Err(e) => {
                        event
                            .adapter
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
                    .adapter
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
                .adapter
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
            // 清空清单需要重写路由文件，故 scope_only = false。
            match ConfigManager::update_warp_routing_rules(
                Vec::new(),
                WarpMode::Default,
                WarpScope::Domain,
                false,
            )
            .await
            {
                Ok(_) => {
                    event
                        .adapter
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
                        .adapter
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
                    .adapter
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
                    .adapter
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
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.restarting").into()),
                )
                .await?;
            match WarpInstaller::restart_service().await {
                Ok(_) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(t!("warp.restart_success").into()),
                        )
                        .await?;
                }
                Err(e) => {
                    event
                        .adapter
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
                .adapter
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
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("warp.uninstalling").into()),
                )
                .await?;
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!("warp.uninstalling").into(),
                        markup: None,
                    },
                )
                .await?;

            match WarpInstaller::uninstall_paired().await {
                Ok(outcome) => {
                    // 文案由**实际卸了什么**决定（4 种组合），
                    // 而不是笼统的「卸载成功」——否则管理员无法确认
                    // 到底清掉了几套实现。
                    event
                        .adapter
                        .send_message(
                            &event.target,
                            MessageContent {
                                text: t!(outcome.i18n_key()).into(),
                                markup: None,
                            },
                        )
                        .await?;
                    Ok(HandlerAction::Redirect("m_warp".to_string()))
                }
                Err(e) => {
                    event
                        .adapter
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：含多字节字符的长规则不得 panic。
    ///
    /// 修复前这里是 `&rule[..27]`（字节切片）。规则来自管理员自由输入且
    /// **无任何校验**，只要第 27 字节落在多字节字符内部就 panic
    /// （`byte index 27 is not a char boundary`）。该路径在每次渲染
    /// 删除菜单时都会执行，因此一个含中文的长规则会让删除按钮永久崩溃。
    #[test]
    fn test_truncate_handles_multibyte_at_boundary() {
        // 26 个 ASCII 字节后接一个 3 字节汉字：第 27 字节正好落在它内部。
        let mut rule = "a".repeat(26);
        rule.push('中');
        rule.push_str("tailtailtail");
        assert!(rule.len() > BUTTON_RULE_MAX_BYTES);
        assert!(!rule.is_char_boundary(BUTTON_RULE_MAX_CHARS));

        // 修复前这一行会 panic。
        let out = truncate_for_button(&rule);
        assert!(out.ends_with("..."), "超长规则应带省略号");
        assert!(out.contains('中'), "首个多字节字符应完整保留");
    }

    /// 全中文长规则同样不得 panic。
    #[test]
    fn test_truncate_handles_all_multibyte() {
        let rule = "域名".repeat(20); // 60 字节
        assert!(rule.len() > BUTTON_RULE_MAX_BYTES);
        let out = truncate_for_button(&rule);
        assert!(out.ends_with("..."));
        // 前缀必须是 27 个**字符**（54 字节），而不是 27 字节。
        assert!(out.starts_with(&"域名".repeat(13)));
    }

    /// 短规则原样返回（只做 HTML 转义）。
    #[test]
    fn test_truncate_leaves_short_rules_intact() {
        let out = truncate_for_button("domain:a.com");
        assert_eq!(out, "domain:a.com");
        assert!(!out.ends_with("..."));
    }

    /// 边界值：恰好 30 字节不截断，31 字节才截断。
    #[test]
    fn test_truncate_boundary_is_exact() {
        let exactly = "a".repeat(BUTTON_RULE_MAX_BYTES);
        assert_eq!(truncate_for_button(&exactly), exactly);

        let over = "a".repeat(BUTTON_RULE_MAX_BYTES + 1);
        assert!(truncate_for_button(&over).ends_with("..."));
    }

    /// HTML 特殊字符仍被转义（不能因按字符截断而丢掉转义）。
    #[test]
    fn test_truncate_still_escapes_html() {
        let out = truncate_for_button("domain:<script>");
        assert!(!out.contains("<script>"), "HTML 必须转义，实际输出: {out}");
    }

    /// 长且含 HTML 的规则：截断与转义都要生效。
    #[test]
    fn test_truncate_escapes_after_truncating() {
        let mut rule = "a".repeat(26);
        rule.push('中');
        rule.push_str("<script>alert(1)</script>");
        let out = truncate_for_button(&rule);
        assert!(out.ends_with("..."));
        assert!(!out.contains("<script>"), "截断后仍必须转义: {out}");
    }
}
