use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::subscription::config::{CertificateMode, GeneratedToken, SubscriptionConfig};
use crate::core::subscription::runtime::{SubscriptionStatus, subscription_runtime};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionInput {
    Domain,
    Ip,
    Ipv6San,
    Port,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    Applied,
    Invalid,
}

pub const SUBSCRIPTION_INPUT_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    match data {
        "m_subscription" => show_subscription_menu(event).await,
        "sub_toggle" => handle_toggle(event).await,
        "sub_mode_domain" => handle_arm_mode(event, SubscriptionInput::Domain).await,
        "sub_mode_ip" => handle_arm_mode(event, SubscriptionInput::Ip).await,
        "sub_set_ipv6" => handle_arm_mode(event, SubscriptionInput::Ipv6San).await,
        "sub_set_port" => handle_arm_mode(event, SubscriptionInput::Port).await,
        "sub_regenerate_token" => handle_regenerate_token(event).await,
        "sub_reissue_certificate" => handle_reissue_certificate(event).await,
        "sub_refresh" => show_subscription_menu(event).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn show_subscription_menu(event: &CallbackEvent) -> HandlerResult {
    let runtime = subscription_runtime();
    if runtime.is_none() {
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("subscription.not_init").into_owned(),
                    markup: Some(Markup {
                        buttons: vec![vec![InlineButton {
                            text: t!("menu.back_settings").into(),
                            data: "m_settings".into(),
                        }]],
                    }),
                },
            )
            .await?;
        return Ok(HandlerAction::Done);
    }

    let runtime = runtime.unwrap();
    let status = runtime.status().await;
    let text = build_subscription_status_text(&status);

    let toggle_text = if status.enabled {
        t!("subscription.disable").into_owned()
    } else {
        t!("subscription.enable").into_owned()
    };

    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: toggle_text,
                data: "sub_toggle".into(),
            }],
            vec![
                InlineButton {
                    text: t!("subscription.set_domain").into(),
                    data: "sub_mode_domain".into(),
                },
                InlineButton {
                    text: t!("subscription.set_ip").into(),
                    data: "sub_mode_ip".into(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("subscription.set_ipv6_san").into(),
                    data: "sub_set_ipv6".into(),
                },
                InlineButton {
                    text: t!("subscription.set_port").into(),
                    data: "sub_set_port".into(),
                },
            ],
            vec![
                InlineButton {
                    text: t!("subscription.regenerate_token_btn").into(),
                    data: "sub_regenerate_token".into(),
                },
                InlineButton {
                    text: t!("subscription.reissue_cert_btn").into(),
                    data: "sub_reissue_certificate".into(),
                },
            ],
            vec![InlineButton {
                text: t!("subscription.refresh").into(),
                data: "sub_refresh".into(),
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
                text: format!("🔐 <b>{}</b>\n\n{text}", t!("subscription.title")),
                markup: Some(markup),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

fn build_subscription_status_text(status: &SubscriptionStatus) -> String {
    let enabled_text = if status.enabled {
        t!("subscription.enabled_status")
    } else {
        t!("subscription.disabled_status")
    };

    let host_display = if status.public_host.is_empty() {
        t!("subscription.not_set").into_owned()
    } else {
        status.public_host.clone()
    };

    let port_display = if status.port == 0 {
        t!("subscription.not_set").into_owned()
    } else {
        status.port.to_string()
    };

    let mode_display = if status.public_host.parse::<IpAddr>().is_ok() {
        t!("subscription.mode_ip")
    } else if !status.public_host.is_empty() {
        t!("subscription.mode_domain")
    } else {
        t!("subscription.not_set")
    };

    let cert_status = match status.certificate_not_after {
        Some(_) => t!("subscription.cert_valid"),
        None => t!("subscription.cert_none"),
    };

    let token_display = if status.masked_token.is_empty() {
        t!("subscription.not_set").into_owned()
    } else {
        status.masked_token.clone()
    };

    let last_error = status
        .last_error
        .as_deref()
        .map(|e| format!("\n\n{} <code>{e}</code>", t!("subscription.last_error")))
        .unwrap_or_default();

    format!(
        "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: <code>{}</code>{}: {}{last_error}",
        t!("subscription.status"),
        enabled_text,
        t!("subscription.mode_label"),
        mode_display,
        t!("subscription.host_label"),
        host_display,
        t!("subscription.port_label"),
        port_display,
        t!("subscription.masked_token_label"),
        token_display,
        t!("subscription.cert_label"),
        cert_status,
    )
}

async fn handle_toggle(event: &CallbackEvent) -> HandlerResult {
    let Some(runtime) = subscription_runtime() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("subscription.not_init").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Redirect("m_subscription".into()));
    };

    let status = runtime.status().await;
    let host_is_ip = status.public_host.parse::<IpAddr>().is_ok();
    let result = if status.enabled {
        runtime.disable().await
    } else {
        let mut config = SubscriptionConfig::new_disabled(&status.token_hash);
        config.public_host = status.public_host;
        config.port = if status.port == 0 { 443 } else { status.port };
        config.certificate_mode = if host_is_ip {
            CertificateMode::Ip
        } else {
            CertificateMode::Domain
        };
        config.enabled = true;
        if config.token_hash.is_empty() {
            let generated = GeneratedToken::new();
            config.token_hash = generated.hash().to_owned();
            let base = config.public_base_url();
            let _ = event
                .adapter
                .send_message(
                    &event.target,
                    MessageContent {
                        text: t!(
                            "subscription.token_regenerated_msg",
                            "0" => format!("{base}/sub/{}", generated.raw()),
                            "1" => format!("{base}/sub/{}/clash", generated.raw())
                        )
                        .into_owned(),
                        markup: None,
                    },
                )
                .await;
        }
        runtime.apply(config).await
    };

    match result {
        Ok(()) => {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("subscription.toggle_success").into_owned()),
                )
                .await?;
        }
        Err(e) => {
            log::error!("subscription toggle failed: {e:#}");
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("subscription.operation_failed").into_owned()),
                )
                .await?;
        }
    }

    Ok(HandlerAction::Redirect("m_subscription".into()))
}

async fn handle_arm_mode(event: &CallbackEvent, input_type: SubscriptionInput) -> HandlerResult {
    let Some(_runtime) = subscription_runtime() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("subscription.not_init").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    // Input already armed by state_ops::intercept — just show the prompt.
    let prompt = match input_type {
        SubscriptionInput::Domain => t!("subscription.input_domain_prompt"),
        SubscriptionInput::Ip => t!("subscription.input_ip_prompt"),
        SubscriptionInput::Ipv6San => t!("subscription.input_ipv6_san_prompt"),
        SubscriptionInput::Port => t!("subscription.input_port_prompt"),
    };

    event
        .adapter
        .send_message(
            &event.target,
            MessageContent {
                text: format!("{prompt}\n\n{}", t!("subscription.input_timeout_hint")),
                markup: None,
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_regenerate_token(event: &CallbackEvent) -> HandlerResult {
    let Some(runtime) = subscription_runtime() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("subscription.not_init").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    match runtime.regenerate_token().await {
        Ok(urls) => {
            event
                .adapter
                .send_message(
                    &event.target,
                    MessageContent {
                        text: t!(
                            "subscription.token_regenerated_msg",
                            "0" => urls.standard,
                            "1" => urls.clash
                        )
                        .into_owned(),
                        markup: None,
                    },
                )
                .await?;
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("subscription.token_regenerated_cb").into_owned()),
                )
                .await?;
        }
        Err(e) => {
            log::error!("subscription token regeneration failed: {e:#}");
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("subscription.operation_failed").into_owned()),
                )
                .await?;
        }
    }

    Ok(HandlerAction::Redirect("m_subscription".into()))
}

async fn handle_reissue_certificate(event: &CallbackEvent) -> HandlerResult {
    let Some(runtime) = subscription_runtime() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("subscription.not_init").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(t!("subscription.cert_reissuing").into_owned()),
        )
        .await?;

    match runtime.reissue_certificate().await {
        Ok(()) => {}
        Err(e) => {
            log::error!("subscription certificate reissue failed: {e:#}");
            event
                .adapter
                .send_message(
                    &event.target,
                    MessageContent {
                        text: t!("subscription.operation_failed").into_owned(),
                        markup: None,
                    },
                )
                .await?;
        }
    }

    Ok(HandlerAction::Redirect("m_subscription".into()))
}

pub async fn process_typed_input(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    input_type: SubscriptionInput,
    config: SubscriptionConfig,
    text: &str,
) -> anyhow::Result<InputOutcome> {
    let runtime = match subscription_runtime() {
        Some(r) => r,
        None => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("subscription.not_init").into_owned(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(InputOutcome::Invalid);
        }
    };

    let trimmed = text.trim();

    let mut candidate = config.clone();
    let valid = match input_type {
        SubscriptionInput::Domain => {
            let domain = trimmed.to_string();
            if domain.is_empty() {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("subscription.invalid_input").into_owned(),
                            markup: None,
                        },
                    )
                    .await?;
                return Ok(InputOutcome::Invalid);
            }
            candidate.certificate_mode = CertificateMode::Domain;
            candidate.public_host = domain;
            true
        }
        SubscriptionInput::Ip => {
            if let Ok(ip) = trimmed.parse::<IpAddr>() {
                candidate.certificate_mode = CertificateMode::Ip;
                candidate.public_host = ip.to_string();
                true
            } else {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("subscription.invalid_ip").into_owned(),
                            markup: None,
                        },
                    )
                    .await?;
                false
            }
        }
        SubscriptionInput::Ipv6San => {
            if trimmed.is_empty() || trimmed == "-" {
                candidate.ipv6_san = None;
                true
            } else if let Ok(ip) = trimmed.parse::<IpAddr>() {
                if ip.is_ipv6() {
                    candidate.ipv6_san = Some(ip);
                    true
                } else {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("subscription.invalid_ipv6").into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                    false
                }
            } else {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("subscription.invalid_ipv6").into_owned(),
                            markup: None,
                        },
                    )
                    .await?;
                false
            }
        }
        SubscriptionInput::Port => {
            if let Ok(port) = trimmed.parse::<u16>() {
                if port == 80 {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("subscription.invalid_port_80").into_owned(),
                                markup: None,
                            },
                        )
                        .await?;
                    false
                } else {
                    candidate.port = port;
                    true
                }
            } else {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("subscription.invalid_port").into_owned(),
                            markup: None,
                        },
                    )
                    .await?;
                false
            }
        }
    };

    if !valid {
        return Ok(InputOutcome::Invalid);
    }

    match runtime.apply(candidate).await {
        Ok(()) => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("subscription.config_updated").into_owned(),
                        markup: None,
                    },
                )
                .await?;
        }
        Err(e) => {
            log::error!("subscription config update failed: {e:#}");
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("subscription.operation_failed").into_owned(),
                        markup: None,
                    },
                )
                .await?;
        }
    }

    Ok(InputOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_input_variants_are_clone_and_eq() {
        let d = SubscriptionInput::Domain;
        assert_eq!(d.clone(), SubscriptionInput::Domain);
        let ip = SubscriptionInput::Ip;
        assert_eq!(ip, SubscriptionInput::Ip);
        let v6 = SubscriptionInput::Ipv6San;
        assert_eq!(v6, SubscriptionInput::Ipv6San);
        let p = SubscriptionInput::Port;
        assert_eq!(p, SubscriptionInput::Port);
    }

    #[tokio::test]
    async fn process_typed_input_returns_invalid_when_runtime_none() {
        let adapter = MockSubscriptionAdapter::default();
        let target = TargetId("test".into());
        let config = SubscriptionConfig::new_disabled("ab".repeat(32));
        let outcome = process_typed_input(
            &adapter,
            &target,
            SubscriptionInput::Ip,
            config,
            "192.0.2.1",
        )
        .await
        .unwrap();
        assert_eq!(outcome, InputOutcome::Invalid);
    }

    #[tokio::test]
    async fn process_typed_input_returns_invalid_for_bad_ip() {
        let adapter = MockSubscriptionAdapter::default();
        let target = TargetId("test".into());
        let config = SubscriptionConfig::new_disabled("ab".repeat(32));
        let outcome = process_typed_input(
            &adapter,
            &target,
            SubscriptionInput::Ip,
            config,
            "not_an_ip",
        )
        .await
        .unwrap();
        assert_eq!(outcome, InputOutcome::Invalid);
    }

    #[tokio::test]
    async fn process_typed_input_returns_invalid_for_empty_domain() {
        let adapter = MockSubscriptionAdapter::default();
        let target = TargetId("test".into());
        let config = SubscriptionConfig::new_disabled("ab".repeat(32));
        let outcome =
            process_typed_input(&adapter, &target, SubscriptionInput::Domain, config, " ")
                .await
                .unwrap();
        assert_eq!(outcome, InputOutcome::Invalid);
    }

    #[tokio::test]
    async fn process_typed_input_returns_invalid_for_port_80() {
        let adapter = MockSubscriptionAdapter::default();
        let target = TargetId("test".into());
        let config = SubscriptionConfig::new_disabled("ab".repeat(32));
        let outcome = process_typed_input(&adapter, &target, SubscriptionInput::Port, config, "80")
            .await
            .unwrap();
        assert_eq!(outcome, InputOutcome::Invalid);
    }

    use crate::adapters::common::{BotAdapter, MessageContent, MessageId, Platform};
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockSubscriptionAdapter {
        sent: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BotAdapter for MockSubscriptionAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            self.sent.lock().unwrap().push(content.text);
            Ok(MessageId("0".into()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn download_file(&self, _file_id: &str) -> anyhow::Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> crate::adapters::common::PlatformCapabilities {
            crate::adapters::common::PlatformCapabilities::TELEGRAM
        }
    }
}
