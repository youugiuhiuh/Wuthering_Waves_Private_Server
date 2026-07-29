use std::sync::Arc;
use tokio::time::{Duration, sleep};

use crate::adapters::common::{
    BotAdapter, InlineButton, Markup, MessageContent, MessageId, TargetId,
};
use crate::app::state::AppState;
use crate::core::security::acme::{CertPaths, XhttpDeployMode};
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::types::{DomainFlowSource, IpVersion};
use crate::core::xray::installer::{RealityInstallOutcome, RealityInstaller};
use crate::core::xray::routing::RoutingManager;
use crate::core::xray::{ConfigManager, KcpMask, Proto};
use crate::shared::handlers::message::provider_credential_guidance;
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::utils;
use rust_i18n::t;
use std::fs;

// ── Standalone TLS xhttp ──────────────────────────────────────────────

pub async fn run_standalone_xhttp_tls(
    domain: String,
    certs: CertPaths,
    _source: DomainFlowSource,
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
) -> anyhow::Result<()> {
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

    let ip_str: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up").into(),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up").into(),
    };

    let _ = adapter
        .send_message(
            &target,
            MessageContent {
                text: t!("xray.gen_progress", "0" => 20, "1" => "TLS", "2" => ip_str.as_str())
                    .into_owned(),
                markup: None,
            },
        )
        .await;

    let res = ConfigManager::batch_create_xhttp_tls_enhanced(&domain, &certs, ip_version).await;

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg =
                t!("xray.tls_batch_done", "0" => result.created_count, "1" => domain.as_str())
                    .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_config_file", "0" => filename)
                ));
            }

            if let Some(backup_file) = result.backup_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_backup_file", "0" => backup_file)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
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
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("xray.gen_fail", "0" => e.to_string()).to_string(),
                        markup: None,
                    },
                )
                .await;
        }
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

async fn show_reality_batch_prompt(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    msg_id: &MessageId,
    proto: Proto,
) -> anyhow::Result<()> {
    let (ip_prefix, title) = match proto {
        Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
        Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    let mut buttons = vec![vec![InlineButton {
        text: "🌐 IPv4 (0.0.0.0)".into(),
        data: format!("{}4", ip_prefix),
    }]];

    if has_ipv6 {
        buttons[0].push(InlineButton {
            text: "🌐 IPv6 (::)".into(),
            data: format!("{}6", ip_prefix),
        });

        if proto == Proto::XHTTP {
            buttons.push(vec![
                InlineButton {
                    text: t!("xray.split_v6_up").into(),
                    data: format!("{}s6", ip_prefix),
                },
                InlineButton {
                    text: t!("xray.split_v4_up").into(),
                    data: format!("{}s4", ip_prefix),
                },
            ]);
        }
    }

    buttons.push(vec![InlineButton {
        text: t!("menu.back_user").into(),
        data: "m_usr".into(),
    }]);

    adapter
        .edit_message(
            target,
            msg_id,
            MessageContent {
                text: t!(
                    "xray.batch_title",
                    "0" => title,
                    "1" => t!("xray.batch_security"),
                    "2" => t!("xray.batch_step_ip")
                )
                .into_owned(),
                markup: Some(Markup { buttons }),
            },
        )
        .await?;
    Ok(())
}

pub async fn show_domain_choice(event: &CallbackEvent, source: DomainFlowSource) -> HandlerResult {
    let source_str = match source {
        DomainFlowSource::Standalone => "standalone",
        DomainFlowSource::OneClick => "one_click",
    };
    let buttons = vec![
        vec![InlineButton {
            text: t!("domain.yes").into(),
            data: format!("xhttp_domain_yes:{}", source_str),
        }],
        vec![InlineButton {
            text: t!("domain.no").into(),
            data: format!("xhttp_domain_no:{}", source_str),
        }],
    ];
    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("domain.prompt").into_owned(),
                markup: Some(Markup { buttons }),
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

fn parse_domain_source(data: &str) -> Option<DomainFlowSource> {
    let source = data
        .strip_prefix("xhttp_domain_yes:")
        .or_else(|| data.strip_prefix("xhttp_domain_no:"))?;
    match source {
        "standalone" => Some(DomainFlowSource::Standalone),
        "one_click" => Some(DomainFlowSource::OneClick),
        _ => None,
    }
}

fn one_click_domain_no_mode(data: &str) -> Option<XhttpDeployMode> {
    data.strip_prefix("xhttp_domain_no:")?;
    (parse_domain_source(data) == Some(DomainFlowSource::OneClick))
        .then_some(XhttpDeployMode::Reality)
}

fn parse_provider_callback(data: &str) -> Option<crate::core::types::DnsProvider> {
    let provider_str = data.strip_prefix("xhttp_domain_provider:")?;
    match provider_str {
        "cloudflare" | "cf" => Some(crate::core::types::DnsProvider::Cloudflare),
        "aliyun" | "ali" => Some(crate::core::types::DnsProvider::Aliyun),
        "dnspod" | "dp" => Some(crate::core::types::DnsProvider::Dnspod),
        "route53" | "aws" => Some(crate::core::types::DnsProvider::Route53),
        _ => None,
    }
}

async fn show_reality_qty_prompt(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    msg_id: &MessageId,
    ip_version: IpVersion,
    proto: Proto,
) -> anyhow::Result<()> {
    let ip_ver_code = match ip_version {
        IpVersion::IPv4 => "4",
        IpVersion::IPv6 => "6",
        IpVersion::SplitStackV6Primary => "s6",
        IpVersion::SplitStackV4Primary => "s4",
    };
    let ip_display = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        IpVersion::SplitStackV6Primary => &t!("xray.split_v6_up"),
        IpVersion::SplitStackV4Primary => &t!("xray.split_v4_up"),
    };

    let (exec_prefix, title) = match proto {
        Proto::Vision => ("u_batch_exec:", "Reality"),
        Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let buttons = vec![
        vec![
            InlineButton {
                text: "1".into(),
                data: format!("{exec_prefix}{ip_ver_code}:1"),
            },
            InlineButton {
                text: "3".into(),
                data: format!("{exec_prefix}{ip_ver_code}:3"),
            },
            InlineButton {
                text: "5".into(),
                data: format!("{exec_prefix}{ip_ver_code}:5"),
            },
        ],
        vec![
            InlineButton {
                text: "10".into(),
                data: format!("{exec_prefix}{ip_ver_code}:10"),
            },
            InlineButton {
                text: "20".into(),
                data: format!("{exec_prefix}{ip_ver_code}:20"),
            },
            InlineButton {
                text: "50".into(),
                data: format!("{exec_prefix}{ip_ver_code}:50"),
            },
        ],
        vec![InlineButton {
            text: t!("menu.back_user").into(),
            data: "m_usr".into(),
        }],
    ];

    adapter
        .edit_message(
            target,
            msg_id,
            MessageContent {
                text: t!(
                    "xray.batch_title",
                    "0" => title,
                    "1" => "",
                    "2" => t!("xray.batch_step_qty", "0" => ip_display)
                )
                .into_owned(),
                markup: Some(Markup { buttons }),
            },
        )
        .await?;
    Ok(())
}

fn trigger_reality_auto_init(adapter: Arc<dyn BotAdapter>, target: TargetId, msg_id: MessageId) {
    tokio::spawn(async move {
        match RealityInstaller::run(adapter.as_ref(), &target, Some(&msg_id)).await {
            Ok(RealityInstallOutcome::AlreadyReady) => {
                let _ = show_reality_batch_prompt(&*adapter, &target, &msg_id, Proto::Vision).await;
            }
            Ok(RealityInstallOutcome::Completed) => {
                let _ = show_reality_batch_prompt(&*adapter, &target, &msg_id, Proto::Vision).await;
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.reality_ready").into_owned(),
                            markup: None,
                        },
                    )
                    .await;
            }
            Ok(RealityInstallOutcome::InProgress) => {}
            Err(e) => {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.reality_init_fail", "0" => e).into_owned(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    });
}

// ── mgmt ─────────────────────────────────────────────────────────────

async fn handle_mgmt(event: &CallbackEvent) -> HandlerResult {
    let inbounds = ConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    let mut rows = Vec::new();

    if inbounds.is_empty() {
        rows.push(vec![
            InlineButton {
                text: t!("xray.batch_reality").into(),
                data: "u_batch_init".into(),
            },
            InlineButton {
                text: t!("xray.batch_xhttp").into(),
                data: "u_xhttp_batch_init".into(),
            },
        ]);
        rows.push(vec![InlineButton {
            text: t!("xray.pq_mgmt").into(),
            data: "m_pq_mgmt".into(),
        }]);
        rows.push(vec![InlineButton {
            text: t!("xray.routing_mgmt_btn").into(),
            data: "m_routing".into(),
        }]);
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.mgmt_no_cfg").into_owned(),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
    } else {
        for (i, path) in inbounds.iter().enumerate() {
            let filename = path.split('/').next_back().unwrap_or("Unknown");
            rows.push(vec![InlineButton {
                text: t!("xray.file_btn", "0" => filename).into(),
                data: format!("u_l:{}", i),
            }]);
        }
        rows.push(vec![InlineButton {
            text: t!("xray.del_mgmt_btn").into(),
            data: "m_del_cfg".into(),
        }]);
        rows.push(vec![
            InlineButton {
                text: t!("xray.batch_reality").into(),
                data: "u_batch_init".into(),
            },
            InlineButton {
                text: t!("xray.batch_xhttp").into(),
                data: "u_xhttp_batch_init".into(),
            },
        ]);
        rows.push(vec![
            InlineButton {
                text: t!("xray.batch_kcp").into(),
                data: "u_kcp_init".into(),
            },
            InlineButton {
                text: t!("xray.pq_mgmt").into(),
                data: "m_pq_mgmt".into(),
            },
            InlineButton {
                text: t!("xray.routing_mgmt_btn").into(),
                data: "m_routing".into(),
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
                    text: t!("xray.mgmt_title").into_owned(),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

async fn handle_pq_mgmt(event: &CallbackEvent) -> HandlerResult {
    let configured = ConfigManager::is_reality_pq_configured();
    let status = if configured {
        t!("xray.pq_status_enabled")
    } else {
        t!("xray.pq_status_disabled")
    };
    let rows = vec![
        vec![InlineButton {
            text: t!("xray.pq_delete").into(),
            data: "m_pq_del".into(),
        }],
        vec![InlineButton {
            text: t!("xray.pq_init").into(),
            data: "m_pq_init".into(),
        }],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_xray_mgmt".into(),
        }],
    ];
    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.pq_title", "0" => status).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_pq_del(event: &CallbackEvent) -> HandlerResult {
    match ConfigManager::delete_reality_pq().await {
        Ok(()) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.pq_del_success").into_owned()),
                )
                .await?;
        }
        Err(e) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.pq_del_fail", "0" => e).into_owned()),
                )
                .await?;
        }
    }
    Ok(HandlerAction::Redirect("m_pq_mgmt".to_string()))
}

async fn handle_pq_init(event: &CallbackEvent) -> HandlerResult {
    match ConfigManager::generate_reality_pq_keys().await {
        Ok(()) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.pq_init_success").into_owned()),
                )
                .await?;
        }
        Err(e) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.pq_init_fail", "0" => e).into_owned()),
                )
                .await?;
        }
    }
    Ok(HandlerAction::Redirect("m_pq_mgmt".to_string()))
}

// ── routing ──────────────────────────────────────────────────────────

async fn handle_routing_menu(event: &CallbackEvent) -> HandlerResult {
    let rules = RoutingManager::get_all_with_status()
        .await
        .map_err(|e| anyhow::anyhow!("获取路由规则失败: {}", e))?;

    let active_count = rules.iter().filter(|(_, enabled)| *enabled).count();
    let mut text = t!("xray.routing_title").to_string();
    text.push_str(&format!(
        "\n\n{}",
        t!("xray.routing_active_count", "count" => active_count.to_string())
    ));

    let mut rows: Vec<Vec<InlineButton>> = rules
        .iter()
        .map(|(def, enabled)| {
            let i18n_key = format!("xray.routing_rule_{}", def.id);
            let name = t!(i18n_key.as_str());
            let icon = if *enabled { "✅" } else { "⬜" };
            vec![InlineButton {
                text: format!("{} {}", icon, name),
                data: format!("routing_toggle:{}", def.id),
            }]
        })
        .collect();

    rows.push(vec![InlineButton {
        text: t!("menu.back").into(),
        data: "m_xray_mgmt".into(),
    }]);

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text,
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_routing_toggle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let rule_id = data.strip_prefix("routing_toggle:").unwrap_or("");
    if rule_id.is_empty() {
        return Ok(HandlerAction::Redirect("m_routing".to_string()));
    }

    match RoutingManager::toggle(rule_id).await {
        Ok(enabled) => {
            let i18n_key = format!("xray.routing_rule_{}", rule_id);
            let name = t!(i18n_key.as_str());
            let msg = if enabled {
                t!("xray.routing_toggled_on", "name" => name)
            } else {
                t!("xray.routing_toggled_off", "name" => name)
            };
            event
                .adapter
                .answer_callback(&event.target, &event.callback_id, Some(msg.into()))
                .await?;
        }
        Err(e) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(format!("{}: {}", t!("xray.routing_reload_failed"), e)),
                )
                .await?;
        }
    }

    Ok(HandlerAction::Redirect("m_routing".to_string()))
}

// ── delete ───────────────────────────────────────────────────────────

async fn handle_del_cfg(event: &CallbackEvent) -> HandlerResult {
    let rows = vec![
        vec![
            InlineButton {
                text: t!("xray.filter_all").into(),
                data: "cfg_filter:all".into(),
            },
            InlineButton {
                text: t!("xray.filter_reality").into(),
                data: "cfg_filter:reality".into(),
            },
            InlineButton {
                text: t!("xray.filter_xhttp").into(),
                data: "cfg_filter:xhttp".into(),
            },
            InlineButton {
                text: t!("xray.filter_kcp").into(),
                data: "cfg_filter:kcp".into(),
            },
        ],
        vec![InlineButton {
            text: t!("xray.del_all").into(),
            data: "cfg_del_all_confirm:all".into(),
        }],
        vec![InlineButton {
            text: t!("xray.del_count").into(),
            data: "cfg_del_count:all".into(),
        }],
        vec![InlineButton {
            text: t!("xray.del_select").into(),
            data: "cfg_del_select:all".into(),
        }],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_xray_mgmt".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.del_title", "0" => t!("xray.filter_all")).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_cfg_filter(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let filter = data.strip_prefix("cfg_filter:").unwrap_or("all");
    let filter_label_val = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let rows = vec![
        vec![
            InlineButton {
                text: t!("xray.filter_all").into(),
                data: "cfg_filter:all".into(),
            },
            InlineButton {
                text: t!("xray.filter_reality").into(),
                data: "cfg_filter:reality".into(),
            },
            InlineButton {
                text: t!("xray.filter_xhttp").into(),
                data: "cfg_filter:xhttp".into(),
            },
            InlineButton {
                text: t!("xray.filter_kcp").into(),
                data: "cfg_filter:kcp".into(),
            },
        ],
        vec![InlineButton {
            text: t!("xray.del_all").into(),
            data: format!("cfg_del_all_confirm:{}", filter),
        }],
        vec![InlineButton {
            text: t!("xray.del_count").into(),
            data: format!("cfg_del_count:{}", filter),
        }],
        vec![InlineButton {
            text: t!("xray.del_select").into(),
            data: format!("cfg_del_select:{}", filter),
        }],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_xray_mgmt".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.del_title", "0" => filter_label_val).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_cfg_del_all_confirm(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let filter = data.strip_prefix("cfg_del_all_confirm:").unwrap_or("all");
    let filter_type_label = match filter {
        "reality" => t!("xray.type_reality"),
        "xhttp" => t!("xray.type_xhttp"),
        "kcp" => t!("xray.type_kcp"),
        _ => t!("xray.type_all"),
    };
    let rows = vec![
        vec![InlineButton {
            text: t!("xray.confirm_clear_btn").into(),
            data: format!("cfg_del_all_exec:{}", filter),
        }],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_del_cfg".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.confirm_del_all", "0" => filter_type_label).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_cfg_del_all_exec(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let filter = data.strip_prefix("cfg_del_all_exec:").unwrap_or("all");
    let count = if filter == "all" {
        ConfigManager::delete_all_configurations()
            .await
            .unwrap_or(0)
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("xray.del_unknown_filter").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Redirect("m_del_cfg".to_string()));
            }
        };
        let files = ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default();
        let count = files.len();
        for f in &files {
            let _ = fs::remove_file(f);
        }
        if count > 0 {
            let _ = MaintenanceManager::reload_core().await;
        }
        count
    };
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("xray.del_success_all", "0" => count).into_owned()),
        )
        .await?;

    Ok(HandlerAction::Redirect("m_del_cfg".to_string()))
}

// ── delete_count ─────────────────────────────────────────────────────

async fn handle_cfg_del_count(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let filter = data.strip_prefix("cfg_del_count:").unwrap_or("all");
    let filter_label = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let rows = vec![
        vec![
            InlineButton {
                text: "10".into(),
                data: format!("cfg_del_exec_count:{}:10", filter),
            },
            InlineButton {
                text: "50".into(),
                data: format!("cfg_del_exec_count:{}:50", filter),
            },
        ],
        vec![
            InlineButton {
                text: "100".into(),
                data: format!("cfg_del_exec_count:{}:100", filter),
            },
            InlineButton {
                text: "500".into(),
                data: format!("cfg_del_exec_count:{}:500", filter),
            },
        ],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "cfg_filter:all".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.del_count_title", "0" => filter_label).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_cfg_del_exec_count(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let n: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };

    let mut file_with_time = Vec::new();
    for f in files {
        if let Ok(meta) = std::fs::metadata(&f)
            && let Ok(time) = meta.modified()
        {
            file_with_time.push((f, time));
        }
    }
    file_with_time.sort_by_key(|a| a.1);

    let to_delete = file_with_time.iter().take(n);
    let mut deleted_count = 0;
    for (f, _) in to_delete {
        if fs::remove_file(f).is_ok() {
            deleted_count += 1;
        }
    }
    if deleted_count > 0 {
        let _ = MaintenanceManager::reload_core().await;
    }
    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("xray.del_success_count", "0" => deleted_count).into_owned()),
        )
        .await?;

    Ok(HandlerAction::Redirect(format!("cfg_del_count:{}", filter)))
}

// ── delete_select ────────────────────────────────────────────────────

async fn handle_cfg_del_select(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let filter = data.strip_prefix("cfg_del_select:").unwrap_or("all");
    let files = if filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };
    let filter_label = match filter {
        "reality" => t!("xray.filter_reality"),
        "xhttp" => t!("xray.filter_xhttp"),
        "kcp" => t!("xray.filter_kcp"),
        _ => t!("xray.filter_all"),
    };
    let mut rows = Vec::new();
    for (i, path) in files.iter().enumerate().take(50) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        rows.push(vec![InlineButton {
            text: format!("🗑 {}", filename),
            data: format!("cfg_del_file:{}:{}", filter, i),
        }]);
    }
    rows.push(vec![InlineButton {
        text: t!("menu.back").into(),
        data: "cfg_filter:all".into(),
    }]);

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.del_select_title", "0" => filter_label).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_cfg_del_file(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };

    if let Some(path) = files.get(idx) {
        let filename = path.split('/').next_back().unwrap_or("Unknown");
        let rows = vec![
            vec![InlineButton {
                text: "⚠️ Confirm Delete".into(),
                data: format!("cfg_del_confirm:{}:{}", filter, idx),
            }],
            vec![InlineButton {
                text: t!("menu.back").into(),
                data: format!("cfg_del_select:{}", filter),
            }],
        ];
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.del_confirm_msg", "0" => utils::escape_html(filename))
                        .into_owned(),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.del_not_found").into_owned()),
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

async fn handle_cfg_del_confirm(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let parts: Vec<&str> = data.split(':').collect();
    let filter = parts.get(1).unwrap_or(&"all");
    let idx: usize = parts.get(2).unwrap_or(&"0").parse().unwrap_or(0);

    let files = if *filter == "all" {
        ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default()
    } else {
        let proto = match *filter {
            "reality" => Proto::Vision,
            "xhttp" => Proto::XHTTP,
            "kcp" => Proto::Kcp,
            _ => Proto::Vision,
        };
        ConfigManager::list_inbound_files_by_proto(proto)
            .await
            .unwrap_or_default()
    };

    if let Err(e) = utils::validate_idx(idx, files.len(), &t!("xray.del_label")) {
        event
            .adapter
            .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
            .await?;
        return Ok(HandlerAction::Done);
    }

    if let Some(path) = files.get(idx) {
        let _ = ConfigManager::delete_specific_configuration(path).await;
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.del_success").into_owned()),
            )
            .await?;
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.del_nonexist").into_owned()),
            )
            .await?;
    }

    Ok(HandlerAction::Redirect(format!(
        "cfg_del_select:{}",
        filter
    )))
}

// ── batch ────────────────────────────────────────────────────────────

async fn handle_batch_init(event: &CallbackEvent) -> HandlerResult {
    if MaintenanceManager::is_reality_base_ready().await {
        show_reality_batch_prompt(&*event.adapter, &event.target, &event.msg_id, Proto::Vision)
            .await?;
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.preparing_reality").into_owned()),
            )
            .await?;
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.init_reality").into_owned(),
                    markup: None,
                },
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
    }
    Ok(HandlerAction::Done)
}

async fn handle_xhttp_batch_init(event: &CallbackEvent) -> HandlerResult {
    show_domain_choice(event, DomainFlowSource::Standalone).await?;
    Ok(HandlerAction::Done)
}

async fn handle_batch_ip_init(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_batch_ip_init:";
    let proto = Proto::Vision;
    let ip_ver_code = data.strip_prefix(prefix).unwrap_or("");
    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };
    show_reality_qty_prompt(
        &*event.adapter,
        &event.target,
        &event.msg_id,
        ip_version,
        proto,
    )
    .await?;
    Ok(HandlerAction::Done)
}

async fn handle_xhttp_batch_ip_init(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_xhttp_batch_ip_init:";
    let proto = Proto::XHTTP;
    let ip_ver_code = data.strip_prefix(prefix).unwrap_or("");
    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };
    show_reality_qty_prompt(
        &*event.adapter,
        &event.target,
        &event.msg_id,
        ip_version,
        proto,
    )
    .await?;
    Ok(HandlerAction::Done)
}

async fn handle_batch_exec(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_batch_exec:";
    let proto = Proto::Vision;
    let parts: Vec<&str> = data
        .strip_prefix(prefix)
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let ip_ver_code = parts[0];
    let n: usize = parts[1].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };

    if !MaintenanceManager::is_reality_base_ready().await {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.base_missing").into_owned()),
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
        return Ok(HandlerAction::Done);
    }

    let ip_str: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up").into(),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up").into(),
    };

    let proto_str = match proto {
        Proto::Vision => "Reality",
        Proto::XHTTP => "XHTTP",
        Proto::Kcp => "KCP",
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(
                t!("xray.gen_progress", "0" => n, "1" => proto_str, "2" => ip_str.as_str())
                    .into_owned(),
            ),
        )
        .await?;

    let res = match proto {
        Proto::Vision => ConfigManager::batch_create_reality_vision_enhanced(n, ip_version).await,
        Proto::XHTTP => ConfigManager::batch_create_xhttp_reality_enhanced(n, ip_version).await,
        Proto::Kcp => unreachable!("KCP uses separate batch handler"),
    };

    let adapter = event.adapter.clone();
    let target = event.target.clone();

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.batch_done",
                "0" => result.created_count,
                "1" => ip_str.as_str()
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_config_file", "0" => filename)
                ));
            }

            if let Some(backup_file) = result.backup_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_backup_file", "0" => backup_file)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
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
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("未找到 Reality 配置文件") {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.master_missing").to_string(),
                            markup: None,
                        },
                    )
                    .await;
                trigger_reality_auto_init(
                    event.adapter.clone(),
                    event.target.clone(),
                    event.msg_id.clone(),
                );
            } else {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.gen_fail", "0" => err_msg).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    }

    Ok(HandlerAction::Done)
}

async fn handle_xhttp_batch_exec(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_xhttp_batch_exec:";
    let proto = Proto::XHTTP;
    let parts: Vec<&str> = data
        .strip_prefix(prefix)
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let ip_ver_code = parts[0];
    let n: usize = parts[1].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };

    if !MaintenanceManager::is_reality_base_ready().await {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.base_missing").into_owned()),
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
        return Ok(HandlerAction::Done);
    }

    let ip_str: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up").into(),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up").into(),
    };

    let proto_str = match proto {
        Proto::Vision => "Reality",
        Proto::XHTTP => "XHTTP",
        Proto::Kcp => "KCP",
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(
                t!("xray.gen_progress", "0" => n, "1" => proto_str, "2" => ip_str.as_str())
                    .into_owned(),
            ),
        )
        .await?;

    let res = match proto {
        Proto::Vision => ConfigManager::batch_create_reality_vision_enhanced(n, ip_version).await,
        Proto::XHTTP => ConfigManager::batch_create_xhttp_reality_enhanced(n, ip_version).await,
        Proto::Kcp => unreachable!("KCP uses separate batch handler"),
    };

    let adapter = event.adapter.clone();
    let target = event.target.clone();

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.batch_done",
                "0" => result.created_count,
                "1" => ip_str.as_str()
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_config_file", "0" => filename)
                ));
            }

            if let Some(backup_file) = result.backup_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.batch_backup_file", "0" => backup_file)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
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
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("未找到 Reality 配置文件") {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.master_missing").to_string(),
                            markup: None,
                        },
                    )
                    .await;
                trigger_reality_auto_init(
                    event.adapter.clone(),
                    event.target.clone(),
                    event.msg_id.clone(),
                );
            } else {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: t!("xray.gen_fail", "0" => err_msg).to_string(),
                            markup: None,
                        },
                    )
                    .await;
            }
        }
    }

    Ok(HandlerAction::Done)
}

// ── batch (KCP) ──────────────────────────────────────────────────────

async fn handle_kcp_init(event: &CallbackEvent) -> HandlerResult {
    let rows = vec![
        vec![
            InlineButton {
                text: t!("xray.kcp_cat_enc").into(),
                data: "u_kcp_cat:enc".into(),
            },
            InlineButton {
                text: t!("xray.kcp_cat_obf").into(),
                data: "u_kcp_cat:obf".into(),
            },
        ],
        vec![
            InlineButton {
                text: t!("xray.kcp_cat_dis").into(),
                data: "u_kcp_cat:dis".into(),
            },
            InlineButton {
                text: t!("xray.kcp_cat_ext").into(),
                data: "u_kcp_cat:ext".into(),
            },
        ],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_xray_mgmt".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.kcp_title").into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_cat(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let cat_code = data.strip_prefix("u_kcp_cat:").unwrap_or("enc");
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("unknown");

    let variants = KcpMask::variants_by_category(cat_code);
    let mut rows: Vec<Vec<InlineButton>> = Vec::with_capacity(variants.len());

    for mask in &variants {
        rows.push(vec![InlineButton {
            text: format!("✅ {}", mask.display_name()),
            data: format!("u_kcp_add:{}", mask.code()),
        }]);
    }

    rows.push(vec![InlineButton {
        text: t!("xray.kcp_back_cat").into(),
        data: "u_kcp_init".into(),
    }]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: format!(
                    "{}\n\n{}",
                    t!("xray.kcp_select_mask", "0" => cat_name),
                    mask_list
                ),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_add(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let code = data.strip_prefix("u_kcp_add:").unwrap_or("ml");
    if code == "rl" {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.kcp_realm_note").into_owned()),
            )
            .await?;
        let m = KcpMask::from_code(code).unwrap();
        let stack_display = format!("1️⃣ {}", m.display_name());
        let rows = vec![
            vec![InlineButton {
                text: t!("xray.kcp_add_more").into(),
                data: format!("u_kcp_more:{}", code),
            }],
            vec![InlineButton {
                text: t!("xray.kcp_done_btn").into(),
                data: format!("u_kcp_done:{}", code),
            }],
            vec![InlineButton {
                text: t!("xray.kcp_clear_btn").into(),
                data: "u_kcp_init".into(),
            }],
        ];
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: format!(
                        "{}\n\n{}",
                        t!("xray.kcp_stack_more", "0" => stack_display),
                        t!("xray.kcp_realm_note")
                    ),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
        return Ok(HandlerAction::Done);
    }
    if let Some(m) = KcpMask::from_code(code) {
        if let Err(e) = m.is_compatible_with(&[]) {
            event
                .adapter
                .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
                .await?;
            return Ok(HandlerAction::Done);
        }
        let stack_display = format!("1️⃣ {}", m.display_name());
        let rows = vec![
            vec![InlineButton {
                text: t!("xray.kcp_add_more").into(),
                data: format!("u_kcp_more:{}", code),
            }],
            vec![InlineButton {
                text: t!("xray.kcp_done_btn").into(),
                data: format!("u_kcp_done:{}", code),
            }],
            vec![InlineButton {
                text: t!("xray.kcp_clear_btn").into(),
                data: "u_kcp_init".into(),
            }],
        ];
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.kcp_stack_more", "0" => stack_display).into_owned(),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

async fn handle_kcp_more(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let existing = data.strip_prefix("u_kcp_more:").unwrap_or("");
    let existing_codes: Vec<&str> = existing.split(',').collect();

    let stack_display: Vec<String> = existing_codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let mut rows: Vec<Vec<InlineButton>> = Vec::new();

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let has_sudoku = current_masks.iter().any(|m| m.is_sudoku());
    let has_encryption = current_masks.iter().any(|m| m.is_encryption());

    let cat_counts = [
        (
            "enc",
            "🔐 Encryption",
            KcpMask::variants_by_category("enc").len(),
        ),
        (
            "obf",
            "🌀 Obfuscation",
            KcpMask::variants_by_category("obf").len(),
        ),
        (
            "ext",
            "⚡ Extension",
            KcpMask::variants_by_category("ext").len(),
        ),
    ];

    for (code, name, total) in &cat_counts {
        let added_count = existing_codes
            .iter()
            .filter(|ec| {
                KcpMask::from_code(ec)
                    .map(|m| m.category_code() == *code)
                    .unwrap_or(false)
            })
            .count();
        let remaining = total - added_count;

        let disabled_reason = match *code {
            "enc" if has_encryption => Some("added"),
            "obf" if has_sudoku => Some("sudoku added"),
            _ => None,
        };

        if let Some(reason) = disabled_reason {
            rows.push(vec![InlineButton {
                text: format!("⛔ {} ({})", name, reason),
                data: "noop".into(),
            }]);
        } else if remaining > 0 {
            if rows.is_empty() || rows.last().unwrap().len() >= 2 {
                rows.push(Vec::new());
            }
            rows.last_mut().unwrap().push(InlineButton {
                text: format!("{} ({})", name, remaining),
                data: format!("u_kcp_mcat:{}:{}", existing, code),
            });
        } else {
            rows.push(vec![InlineButton {
                text: format!("⛔ {} (max reached)", name),
                data: "noop".into(),
            }]);
        }
    }

    rows.push(vec![InlineButton {
        text: t!("xray.kcp_done_btn").into(),
        data: format!("u_kcp_done:{}", existing),
    }]);
    rows.push(vec![InlineButton {
        text: t!("xray.kcp_clear_btn").into(),
        data: "u_kcp_init".into(),
    }]);

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!(
                    "xray.kcp_select_cat_stack",
                    "0" => stack_display.join("\n"),
                    "1" => existing_codes.len()
                )
                .into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_mcat(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let rest = data.strip_prefix("u_kcp_mcat:").unwrap_or("");
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let existing = parts[0];
    let cat_code = parts[1];
    let existing_codes: Vec<&str> = existing.split(',').collect();
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("unknown");

    let variants = KcpMask::variants_by_category(cat_code);

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let stack_display: Vec<String> = existing_codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let mut rows: Vec<Vec<InlineButton>> = Vec::with_capacity(variants.len());

    for mask in &variants {
        let code = mask.code();
        if existing_codes.contains(&code) {
            rows.push(vec![InlineButton {
                text: format!("☑️ {}", mask.display_name()),
                data: "noop".into(),
            }]);
        } else {
            match mask.is_compatible_with(&current_masks) {
                Ok(()) => {
                    rows.push(vec![InlineButton {
                        text: format!("✅ {}", mask.display_name()),
                        data: format!("u_kcp_push:{}:{}", existing, code),
                    }]);
                }
                Err(e) => {
                    rows.push(vec![InlineButton {
                        text: format!("⛔ {} ({})", mask.display_name(), e),
                        data: format!("noop:⛔:{}", code),
                    }]);
                }
            }
        }
    }

    rows.push(vec![InlineButton {
        text: t!("xray.kcp_back_cat").into(),
        data: format!("u_kcp_more:{}", existing),
    }]);
    rows.push(vec![InlineButton {
        text: t!("xray.kcp_done_btn").into(),
        data: format!("u_kcp_done:{}", existing),
    }]);
    rows.push(vec![InlineButton {
        text: t!("xray.kcp_clear_btn").into(),
        data: "u_kcp_init".into(),
    }]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("<b>{}</b>\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: format!(
                    "{}\n{}\n\n{}\n\n{}",
                    t!("xray.kcp_current_stack"),
                    stack_display.join("\n"),
                    t!("xray.kcp_select_mask", "0" => cat_name),
                    mask_list
                ),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_push(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let rest = data.strip_prefix("u_kcp_push:").unwrap_or("");
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let existing = parts[0];
    let new_code = parts[1];

    let existing_codes: Vec<&str> = if existing.is_empty() {
        vec![]
    } else {
        existing.split(',').collect()
    };

    let current_masks: Vec<KcpMask> = existing_codes
        .iter()
        .filter(|c| !c.is_empty())
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let new_mask = match KcpMask::from_code(new_code) {
        Some(m) => m,
        None => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.kcp_unknown_type").into_owned()),
                )
                .await?;
            return Ok(HandlerAction::Done);
        }
    };

    if let Err(e) = new_mask.is_compatible_with(&current_masks) {
        event
            .adapter
            .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
            .await?;
        return Ok(HandlerAction::Done);
    }

    let new_stack = if existing.is_empty() {
        new_code.to_string()
    } else {
        format!("{},{}", existing, new_code)
    };
    let codes: Vec<&str> = new_stack.split(',').collect();

    let stack_display: Vec<String> = codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let rows = vec![
        vec![InlineButton {
            text: t!("xray.kcp_add_more").into(),
            data: format!("u_kcp_more:{}", new_stack),
        }],
        vec![InlineButton {
            text: t!("xray.kcp_done_btn").into(),
            data: format!("u_kcp_done:{}", new_stack),
        }],
        vec![InlineButton {
            text: t!("xray.kcp_clear_btn").into(),
            data: "u_kcp_init".into(),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("xray.kcp_stack_more", "0" => stack_display.join("\n")).into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_done(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let mask_codes_str = data.strip_prefix("u_kcp_done:").unwrap_or("");
    let codes: Vec<&str> = mask_codes_str.split(',').collect();

    if codes.is_empty() {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.kcp_min_one").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    }

    let masks: Vec<KcpMask> = codes.iter().filter_map(|c| KcpMask::from_code(c)).collect();

    let ordered = KcpMask::canonical_order(&masks);

    if let Err(e) = KcpMask::validate_stack(&ordered) {
        event
            .adapter
            .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
            .await?;
        return Ok(HandlerAction::Done);
    }

    let warnings = KcpMask::get_stack_warnings(&ordered);
    let stack_display: Vec<String> = ordered
        .iter()
        .map(|m| m.display_name().to_string())
        .collect();

    let ordered_codes: Vec<String> = ordered.iter().map(|m| m.code().to_string()).collect();
    let ordered_str = ordered_codes.join(",");

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
    let mut rows = vec![vec![InlineButton {
        text: "🌐 IPv4 (0.0.0.0)".into(),
        data: format!("u_kcp_ip:{}:4", ordered_str),
    }]];
    if has_ipv6 {
        rows[0].push(InlineButton {
            text: "🌐 IPv6 (::)".into(),
            data: format!("u_kcp_ip:{}:6", ordered_str),
        });
    }
    rows.push(vec![InlineButton {
        text: t!("xray.dual_v4").into(),
        data: format!("u_kcp_ip:{}:s4", ordered_str),
    }]);
    rows.push(vec![InlineButton {
        text: t!("xray.dual_v6").into(),
        data: format!("u_kcp_ip:{}:s6", ordered_str),
    }]);
    rows.push(vec![InlineButton {
        text: t!("menu.back").into(),
        data: format!("u_kcp_more:{}", mask_codes_str),
    }]);

    let warning_text = if warnings.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", warnings.join("\n"))
    };

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!(
                    "xray.kcp_stack_config",
                    "0" => stack_display.join(" → "),
                    "1" => warning_text,
                    "2" => t!("xray.batch_step_ip")
                )
                .into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_ip(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let rest = data.strip_prefix("u_kcp_ip:").unwrap_or("");
    let last_colon = rest.rfind(':').unwrap_or(rest.len());
    let mask_codes_str = &rest[..last_colon];
    let ip_ver_code = &rest[last_colon + 1..];
    let codes: Vec<&str> = mask_codes_str.split(',').collect();

    let ip_version: IpVersion = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s4" => IpVersion::SplitStackV4Primary,
        "s6" => IpVersion::SplitStackV6Primary,
        _ => IpVersion::IPv4,
    };
    let ip_display: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV4Primary => t!("xray.dual_v4").into(),
        IpVersion::SplitStackV6Primary => t!("xray.dual_v6").into(),
    };

    let stack_display: Vec<String> = codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let m = KcpMask::from_code(c);
            format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
        })
        .collect();

    let rows = vec![
        vec![
            InlineButton {
                text: "1".into(),
                data: format!("u_kcp_ok:{}:{}:1", mask_codes_str, ip_ver_code),
            },
            InlineButton {
                text: "3".into(),
                data: format!("u_kcp_ok:{}:{}:3", mask_codes_str, ip_ver_code),
            },
            InlineButton {
                text: "5".into(),
                data: format!("u_kcp_ok:{}:{}:5", mask_codes_str, ip_ver_code),
            },
        ],
        vec![
            InlineButton {
                text: "10".into(),
                data: format!("u_kcp_ok:{}:{}:10", mask_codes_str, ip_ver_code),
            },
            InlineButton {
                text: "20".into(),
                data: format!("u_kcp_ok:{}:{}:20", mask_codes_str, ip_ver_code),
            },
            InlineButton {
                text: "50".into(),
                data: format!("u_kcp_ok:{}:{}:50", mask_codes_str, ip_ver_code),
            },
        ],
        vec![InlineButton {
            text: t!("menu.back").into(),
            data: format!("u_kcp_done:{}", mask_codes_str),
        }],
    ];

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!(
                    "xray.kcp_batch_title",
                    "0" => stack_display.join("\n"),
                    "1" => ip_display,
                    "2" => "⬇️ <b>Please select quantity:</b>"
                )
                .into_owned(),
                markup: Some(Markup { buttons: rows }),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_kcp_ok(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let rest = data.strip_prefix("u_kcp_ok:").unwrap_or("");
    let parts: Vec<&str> = rest.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let n: usize = parts[0].parse().unwrap_or(0);
    let remaining = parts[1];
    let last_colon = remaining.rfind(':').unwrap_or(remaining.len());
    let mask_codes_str = &remaining[..last_colon];
    let ip_ver_code = &remaining[last_colon + 1..];

    let ip_version: IpVersion = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s4" => IpVersion::SplitStackV4Primary,
        "s6" => IpVersion::SplitStackV6Primary,
        _ => IpVersion::IPv4,
    };
    let ip_str: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV4Primary => t!("xray.dual_v4").into(),
        IpVersion::SplitStackV6Primary => t!("xray.dual_v6").into(),
    };

    let mask_codes: Vec<&str> = mask_codes_str.split(',').collect();

    let mask_names: Vec<&str> = mask_codes
        .iter()
        .filter_map(|c| KcpMask::from_code(c).map(|m| m.display_name()))
        .collect();
    let mask_label = mask_names.join("+");

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("xray.gen_kcp_progress", "0" => n).into_owned()),
        )
        .await?;

    let res = ConfigManager::batch_create_kcp(n, ip_version, &mask_codes).await;

    let adapter = event.adapter.clone();
    let target = event.target.clone();

    match res {
        Ok(result) => {
            let mut message_ids: Vec<String> = Vec::with_capacity(result.links.len());

            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty()
                && let Ok(msg) = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await
            {
                message_ids.push(msg.0);
            }

            let mut result_msg = t!(
                "xray.kcp_batch_done",
                "0" => result.created_count,
                "1" => ip_str.as_str(),
                "2" => mask_label
            )
            .into_owned();

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!(
                    "\n\n{}",
                    t!("xray.kcp_config_file", "0" => filename)
                ));
            }

            if let Ok(msg) = adapter
                .send_message(
                    &target,
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
                        log::warn!("删除消息失败: {}", e);
                    }
                }
            });
        }
        Err(e) => {
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("xray.gen_fail", "0" => e).to_string(),
                        markup: None,
                    },
                )
                .await;
        }
    }

    Ok(HandlerAction::Done)
}

// ── batch (user list/delete) ─────────────────────────────────────────

async fn handle_user_list(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let idx: usize = data
        .strip_prefix("u_l:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let inbounds = ConfigManager::list_all_inbound_files()
        .await
        .unwrap_or_default();
    if let Err(e) = utils::validate_idx(idx, inbounds.len(), &t!("xray.user_label")) {
        event
            .adapter
            .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
            .await?;
        return Ok(HandlerAction::Done);
    }
    if let Some(path) = inbounds.get(idx) {
        let clients = ConfigManager::get_clients_from_config(path)
            .await
            .unwrap_or_default();
        let mut rows = Vec::new();
        for client in clients {
            let email = client["email"]
                .as_str()
                .or(client["name"].as_str())
                .unwrap_or("Unknown");
            rows.push(vec![InlineButton {
                text: format!("👤 {}", email),
                data: format!("u_d:{}:{}", idx, email),
            }]);
        }
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
                    text: t!(
                        "xray.user_list_title",
                        "0" => path.split('/').next_back().unwrap_or("Unknown")
                    )
                    .into_owned(),
                    markup: Some(Markup { buttons: rows }),
                },
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

async fn handle_user_del(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let parts: Vec<&str> = data
        .strip_prefix("u_d:")
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() == 2 {
        let idx: usize = parts[0].parse().unwrap_or(0);
        let email = parts[1];
        let inbounds = ConfigManager::list_all_inbound_files()
            .await
            .unwrap_or_default();

        if inbounds.get(idx).is_some() {
            let rows = vec![
                vec![InlineButton {
                    text: "⚠️ Confirm Delete".into(),
                    data: format!("u_d_confirm:{}:{}", idx, email),
                }],
                vec![InlineButton {
                    text: t!("menu.back").into(),
                    data: format!("u_l:{}", idx),
                }],
            ];
            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "xray.user_del_confirm",
                            "0" => utils::escape_html(email)
                        )
                        .into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;
        } else {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("xray.user_cfg_not_found").into_owned()),
                )
                .await?;
        }
    }

    Ok(HandlerAction::Done)
}

async fn handle_user_del_confirm(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let parts: Vec<&str> = data
        .strip_prefix("u_d_confirm:")
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() == 2 {
        let email = parts[1];
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.user_del_not_supported", "0" => email).into_owned()),
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

// ── Main dispatch ─────────────────────────────────────────────────────

pub async fn handle(event: &CallbackEvent, state: &AppState) -> HandlerResult {
    let data = event.data.as_str();
    match data {
        "m_xray_mgmt" => handle_mgmt(event).await,
        "m_pq_mgmt" => handle_pq_mgmt(event).await,
        "m_pq_del" => handle_pq_del(event).await,
        "m_pq_init" => handle_pq_init(event).await,

        "m_del_cfg" => handle_del_cfg(event).await,
        d if d.starts_with("cfg_filter:") => handle_cfg_filter(event).await,
        d if d == "cfg_del_all_confirm" || d.starts_with("cfg_del_all_confirm:") => {
            handle_cfg_del_all_confirm(event).await
        }
        d if d == "cfg_del_all_exec" || d.starts_with("cfg_del_all_exec:") => {
            handle_cfg_del_all_exec(event).await
        }

        d if d == "cfg_del_count" || d.starts_with("cfg_del_count:") => {
            handle_cfg_del_count(event).await
        }
        d if d.starts_with("cfg_del_exec_count:") => handle_cfg_del_exec_count(event).await,

        d if d == "cfg_del_select" || d.starts_with("cfg_del_select:") => {
            handle_cfg_del_select(event).await
        }
        d if d.starts_with("cfg_del_file:") => handle_cfg_del_file(event).await,
        d if d.starts_with("cfg_del_confirm:") => handle_cfg_del_confirm(event).await,

        "a_inst_base" | "u_batch_init" => handle_batch_init(event).await,
        d if d.starts_with("u_batch_ip_init:") => handle_batch_ip_init(event).await,
        d if d.starts_with("u_batch_exec:") => handle_batch_exec(event).await,
        "u_xhttp_batch_init" => handle_xhttp_batch_init(event).await,
        d if d.starts_with("u_xhttp_batch_ip_init:") => handle_xhttp_batch_ip_init(event).await,
        d if d.starts_with("u_xhttp_batch_exec:") => handle_xhttp_batch_exec(event).await,
        "u_kcp_init" => handle_kcp_init(event).await,
        d if d.starts_with("u_kcp_cat:") => handle_kcp_cat(event).await,
        d if d.starts_with("u_kcp_add:") => handle_kcp_add(event).await,
        d if d.starts_with("u_kcp_more:") => handle_kcp_more(event).await,
        d if d.starts_with("u_kcp_mcat:") => handle_kcp_mcat(event).await,
        d if d.starts_with("u_kcp_push:") => handle_kcp_push(event).await,
        d if d.starts_with("u_kcp_done:") => handle_kcp_done(event).await,
        d if d.starts_with("u_kcp_ip:") => handle_kcp_ip(event).await,
        d if d.starts_with("u_kcp_ok:") => handle_kcp_ok(event).await,
        d if d.starts_with("u_l:") => handle_user_list(event).await,
        d if d.starts_with("u_d:") => handle_user_del(event).await,
        d if d.starts_with("u_d_confirm:") => handle_user_del_confirm(event).await,

        "m_routing" => handle_routing_menu(event).await,
        d if d.starts_with("routing_toggle:") => handle_routing_toggle(event).await,

        d if d.starts_with("xhttp_domain_yes:") => handle_domain_yes(event, state, d).await,
        d if d.starts_with("xhttp_domain_no:") => handle_domain_no(event, d).await,
        d if d.starts_with("xhttp_domain_provider:") => {
            handle_domain_provider(event, state, d).await
        }

        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_domain_yes(event: &CallbackEvent, state: &AppState, data: &str) -> HandlerResult {
    let source = parse_domain_source(data);
    if source.is_none() {
        return Ok(HandlerAction::Done);
    }
    let source = source.unwrap();
    state
        .start_domain_input(event.target.0.clone(), source, std::time::Instant::now())
        .await;
    event
        .adapter
        .send_message(
            &event.target,
            MessageContent {
                text: t!("domain.input_prompt").into_owned(),
                markup: None,
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

async fn handle_domain_no(event: &CallbackEvent, data: &str) -> HandlerResult {
    if let Some(mode) = one_click_domain_no_mode(data) {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("ops.deploy_start").into_owned()),
            )
            .await?;
        let event = CallbackEvent {
            adapter: event.adapter.clone(),
            target: event.target.clone(),
            user_id: event.user_id.clone(),
            msg_id: event.msg_id.clone(),
            data: "a_one_click_reality".into(),
            callback_id: event.callback_id.clone(),
            session_timeout_secs: event.session_timeout_secs,
        };
        tokio::spawn(async move {
            if let Err(e) = crate::shared::handlers::ops::run_one_click(event, (), mode).await {
                log::error!("run_one_click Reality failed: {}", e);
            }
        });
        return Ok(HandlerAction::Done);
    }

    if MaintenanceManager::is_reality_base_ready().await {
        show_reality_batch_prompt(&*event.adapter, &event.target, &event.msg_id, Proto::XHTTP)
            .await?;
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.preparing_reality").into_owned()),
            )
            .await?;
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.init_reality").into_owned(),
                    markup: None,
                },
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
    }
    Ok(HandlerAction::Done)
}

async fn handle_domain_provider(
    event: &CallbackEvent,
    state: &AppState,
    data: &str,
) -> HandlerResult {
    let provider = parse_provider_callback(data);
    if provider.is_none() {
        return Ok(HandlerAction::Done);
    }
    let provider = provider.unwrap();
    let target_str = &event.target.0;
    let transitioned = state
        .transition_domain_input(
            target_str,
            crate::core::types::DomainInputStep::AwaitProvider,
            crate::core::types::DomainInputStep::AwaitCredentials(provider),
            None,
        )
        .await;
    if !transitioned {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("domain.flow_expired").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    }
    event
        .adapter
        .send_message(
            &event.target,
            MessageContent {
                text: provider_credential_guidance(provider),
                markup: None,
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_click_domain_no_selects_reality_backend() {
        assert!(matches!(
            one_click_domain_no_mode("xhttp_domain_no:one_click"),
            Some(XhttpDeployMode::Reality)
        ));
        assert!(one_click_domain_no_mode("xhttp_domain_no:standalone").is_none());
        assert!(one_click_domain_no_mode("xhttp_domain_maybe:one_click").is_none());
    }
}
