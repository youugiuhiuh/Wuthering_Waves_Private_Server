use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::app::state::{DomainInputState, DomainInputStep};
use crate::core::security::acme::CertPaths;
use crate::core::xray::config::ConfigManager;
use crate::shared::types::TimeoutStatus;

const MAX_INPUT_LENGTH: usize = 4096;

pub enum MessageAction {
    Handled,
    NeedsDestruct,
}

#[async_trait]
pub trait MessageState: Send + Sync {
    async fn schedule_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn remove_schedule_input(&self, chat_id: &str);
    async fn take_warp_input_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn has_pending_domain_input(&self, chat_id: &str) -> bool;
    async fn take_domain_input(&self, chat_id: &str) -> Option<DomainInputState>;
    async fn start_domain_input(&self, chat_id: String);
    async fn start_domain_input_with(&self, chat_id: String, domain: String, step: DomainInputStep);
}

pub async fn handle_message(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    text: Option<&str>,
    has_file: bool,
    state: &dyn MessageState,
) -> anyhow::Result<MessageAction> {
    // Input length check
    if let Some(t) = text
        && t.len() > MAX_INPUT_LENGTH
    {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("message.input_too_long", "0" => MAX_INPUT_LENGTH.to_string())
                        .to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    let target_str = &target.0;

    // Schedule timeout check
    match state
        .schedule_timeout_status(target_str, Duration::from_secs(180))
        .await
    {
        TimeoutStatus::Expired => {
            state.remove_schedule_input(target_str).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("schedule.input_timeout").to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::Active => {
            if text.is_some() || has_file {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("schedule.input_prompt").to_string(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::NotTracked => {}
    }

    // Warp input check
    match state
        .take_warp_input_status(target_str, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("message.warp_input_timeout").to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::Active => {
            if let Some(t) = text {
                let rules: Vec<String> = t
                    .split([',', '，', '\n'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if rules.is_empty() {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("message.warp_input_empty").to_string(),
                                markup: None,
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("message.warp_rule_added").to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                    Err(e) => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("message.warp_add_fail", "0" => e.to_string())
                                        .to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                }
            }
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::NotTracked => {}
    }

    // Domain input check
    if state.has_pending_domain_input(target_str).await {
        return handle_domain_input(adapter, target, target_str, text, state).await;
    }

    Ok(MessageAction::NeedsDestruct)
}

async fn handle_domain_input(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    chat_id: &str,
    text: Option<&str>,
    state: &dyn MessageState,
) -> anyhow::Result<MessageAction> {
    let current = match state.take_domain_input(chat_id).await {
        Some(s) => s,
        None => return Ok(MessageAction::NeedsDestruct),
    };

    match current.step {
        DomainInputStep::AwaitDomain => {
            let domain = match text {
                Some(t) => t.trim().to_lowercase(),
                None => {
                    state.start_domain_input(chat_id.to_string()).await;
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: "请输入你的域名，例如 example.com".into(),
                                markup: None,
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }
            };
            if domain.is_empty() || !domain.contains('.') {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "域名格式无效，请重新输入（如 example.com）".into(),
                            markup: None,
                        },
                    )
                    .await?;
                state.start_domain_input(chat_id.to_string()).await;
                return Ok(MessageAction::Handled);
            }

            if crate::core::security::acme::cert_valid(&domain) {
                let cert = CertPaths::for_domain(&domain);
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "证书已存在，正在生成 20 条 TLS XHTTP 配置...".to_string(),
                            markup: None,
                        },
                    )
                    .await?;
                let ip_ver = crate::shared::handlers::xray::ip_version().await;
                crate::shared::handlers::xray::do_tls_batch(
                    adapter, target, "", 20, ip_ver, &domain, &cert,
                )
                .await?;
                return Ok(MessageAction::Handled);
            }

            if let Some(provider) = crate::core::security::acme::detect_dns_provider() {
                if crate::core::security::acme::has_credentials(provider) {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: format!(
                                    "正在为 {} 申请证书（{}）...",
                                    domain,
                                    provider.display_name()
                                ),
                                markup: None,
                            },
                        )
                        .await?;
                    match crate::core::security::acme::issue_cert(
                        &domain,
                        &format!("admin@{}", domain),
                        provider,
                    )
                    .await
                    {
                        Ok(()) => {
                            let cert = CertPaths::for_domain(&domain);
                            adapter
                                .send_message(
                                    target,
                                    MessageContent {
                                        text: "证书申请成功，正在生成配置...".into(),
                                        markup: None,
                                    },
                                )
                                .await?;
                            let ip_ver = crate::shared::handlers::xray::ip_version().await;
                            crate::shared::handlers::xray::do_tls_batch(
                                adapter, target, "", 20, ip_ver, &domain, &cert,
                            )
                            .await?;
                        }
                        Err(e) => {
                            adapter
                                .send_message(
                                    target,
                                    MessageContent {
                                        text: format!("证书申请失败: {}\n请检查 DNS 凭据后重试", e),
                                        markup: None,
                                    },
                                )
                                .await?;
                        }
                    }
                } else {
                    let buttons = vec![
                        vec![
                            InlineButton {
                                text: "Cloudflare".into(),
                                data: "xhttp_tls_prov:cf".into(),
                            },
                            InlineButton {
                                text: "阿里云".into(),
                                data: "xhttp_tls_prov:ali".into(),
                            },
                        ],
                        vec![
                            InlineButton {
                                text: "DNSPod".into(),
                                data: "xhttp_tls_prov:dp".into(),
                            },
                            InlineButton {
                                text: "Route53".into(),
                                data: "xhttp_tls_prov:aws".into(),
                            },
                        ],
                    ];
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: "请选择 DNS 提供商:".into(),
                                markup: Some(Markup { buttons }),
                            },
                        )
                        .await?;
                    state
                        .start_domain_input_with(
                            chat_id.to_string(),
                            domain,
                            DomainInputStep::AwaitProvider,
                        )
                        .await;
                }
            } else {
                let buttons = vec![
                    vec![
                        InlineButton {
                            text: "Cloudflare".into(),
                            data: "xhttp_tls_prov:cf".into(),
                        },
                        InlineButton {
                            text: "阿里云".into(),
                            data: "xhttp_tls_prov:ali".into(),
                        },
                    ],
                    vec![
                        InlineButton {
                            text: "DNSPod".into(),
                            data: "xhttp_tls_prov:dp".into(),
                        },
                        InlineButton {
                            text: "Route53".into(),
                            data: "xhttp_tls_prov:aws".into(),
                        },
                    ],
                ];
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "请选择 DNS 提供商:".into(),
                            markup: Some(Markup { buttons }),
                        },
                    )
                    .await?;
                state
                    .start_domain_input_with(
                        chat_id.to_string(),
                        domain,
                        DomainInputStep::AwaitProvider,
                    )
                    .await;
            }
            Ok(MessageAction::Handled)
        }
        DomainInputStep::AwaitProvider => {
            state
                .start_domain_input_with(
                    chat_id.to_string(),
                    current.domain.unwrap_or_default(),
                    DomainInputStep::AwaitProvider,
                )
                .await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: "请选择 DNS 提供商:".into(),
                        markup: None,
                    },
                )
                .await?;
            Ok(MessageAction::Handled)
        }
        DomainInputStep::AwaitCredentials(provider) => {
            let creds = match text {
                Some(t) => t,
                None => {
                    state
                        .start_domain_input_with(
                            chat_id.to_string(),
                            current.domain.unwrap_or_default(),
                            DomainInputStep::AwaitCredentials(provider),
                        )
                        .await;
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: format!(
                                    "请输入 {} 的 API Token 和 Key（格式: TOKEN,KEY）",
                                    provider.display_name()
                                ),
                                markup: None,
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }
            };
            let parts: Vec<&str> = creds.split(',').map(|s| s.trim()).collect();
            if parts.len() < 2 {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "格式错误，请输入: TOKEN,KEY".into(),
                            markup: None,
                        },
                    )
                    .await?;
                state
                    .start_domain_input_with(
                        chat_id.to_string(),
                        current.domain.unwrap_or_default(),
                        DomainInputStep::AwaitCredentials(provider),
                    )
                    .await;
                return Ok(MessageAction::Handled);
            }
            let domain = current.domain.unwrap_or_default();
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!(
                            "正在为 {} 申请证书（{}）...",
                            domain,
                            provider.display_name()
                        ),
                        markup: None,
                    },
                )
                .await?;
            match crate::core::security::acme::setup_and_issue(
                &domain, provider, parts[0], parts[1],
            )
            .await
            {
                Ok(()) => {
                    let cert = CertPaths::for_domain(&domain);
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: "证书申请成功，正在生成配置...".into(),
                                markup: None,
                            },
                        )
                        .await?;
                    let ip_ver = crate::shared::handlers::xray::ip_version().await;
                    crate::shared::handlers::xray::do_tls_batch(
                        adapter, target, "", 20, ip_ver, &domain, &cert,
                    )
                    .await?;
                }
                Err(e) => {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: format!("证书申请失败: {}", e),
                                markup: None,
                            },
                        )
                        .await?;
                }
            }
            Ok(MessageAction::Handled)
        }
        DomainInputStep::Processing => Ok(MessageAction::Handled),
    }
}
