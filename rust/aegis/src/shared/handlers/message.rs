use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::adapters::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::security::acme::{AcmeManager, XhttpDeployMode};
use crate::core::types::{DnsProvider, DomainFlowSource, DomainInputState, DomainInputStep};
use crate::core::xray::config::ConfigManager;
use crate::shared::types::TimeoutStatus;

const MAX_INPUT_LENGTH: usize = 4096;

pub enum MessageAction {
    Handled,
    NeedsDestruct,
    DomainReady {
        source: DomainFlowSource,
        mode: XhttpDeployMode,
    },
}

#[async_trait]
pub trait MessageState: Send + Sync {
    async fn schedule_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn remove_schedule_input(&self, chat_id: &str);
    async fn take_warp_input_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn start_domain_input(
        &self,
        chat_id: String,
        source: DomainFlowSource,
        now: std::time::Instant,
    );
    async fn domain_input_snapshot(&self, chat_id: &str) -> Option<DomainInputState>;
    async fn transition_domain_input(
        &self,
        chat_id: &str,
        expected: DomainInputStep,
        next: DomainInputStep,
        domain: Option<String>,
    ) -> bool;
    async fn take_domain_input(&self, chat_id: &str) -> Option<DomainInputState>;
    async fn domain_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
}

pub async fn handle_message(
    adapter: &dyn BotAdapter,
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

    if let Some(domain_state) = state.domain_input_snapshot(target_str).await {
        match domain_state.step {
            DomainInputStep::AwaitDomain => {
                match state
                    .domain_timeout_status(target_str, Duration::from_secs(120))
                    .await
                {
                    TimeoutStatus::Expired => {
                        state.take_domain_input(target_str).await;
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("domain.input_timeout").to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                        return Ok(MessageAction::Handled);
                    }
                    TimeoutStatus::Active => {}
                    TimeoutStatus::NotTracked => {}
                }

                if let Some(input) = text {
                    let trimmed = input.trim();
                    if trimmed.is_empty() {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("domain.input_empty").to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                        return Ok(MessageAction::Handled);
                    }

                    match AcmeManager::validate_domain(trimmed) {
                        Ok(domain) => {
                            if let Some(cert_paths) = AcmeManager::cert_valid(&domain).await {
                                state.take_domain_input(target_str).await;
                                return Ok(MessageAction::DomainReady {
                                    source: domain_state.source,
                                    mode: XhttpDeployMode::Tls { domain, cert_paths },
                                });
                            }

                            if let Some(provider) = AcmeManager::configured_provider()
                                && state
                                    .transition_domain_input(
                                        target_str,
                                        DomainInputStep::AwaitDomain,
                                        DomainInputStep::Processing,
                                        Some(domain.clone()),
                                    )
                                    .await
                            {
                                let bin_path = AcmeManager::ensure_installed().await;
                                let cert_exists = matches!(
                                    &bin_path,
                                    Ok(path) if path.is_file()
                                        && AcmeManager::cert_paths(&domain).is_ok()
                                );
                                let msg = if cert_exists {
                                    t!("domain.cert_renew").to_string()
                                } else {
                                    t!("domain.issuing_cert").to_string()
                                };
                                adapter
                                    .send_message(
                                        target,
                                        MessageContent {
                                            text: msg,
                                            markup: None,
                                        },
                                    )
                                    .await?;
                                match AcmeManager::issue_cert(&domain, provider, None).await {
                                    Ok(cert_paths) => {
                                        state.take_domain_input(target_str).await;
                                        return Ok(MessageAction::DomainReady {
                                            source: domain_state.source,
                                            mode: XhttpDeployMode::Tls { domain, cert_paths },
                                        });
                                    }
                                    Err(e) => {
                                        state
                                            .transition_domain_input(
                                                target_str,
                                                DomainInputStep::Processing,
                                                DomainInputStep::AwaitDomain,
                                                None,
                                            )
                                            .await;
                                        adapter
                                            .send_message(
                                                target,
                                                MessageContent {
                                                    text:
                                                        t!("domain.cert_fail", "0" => e.to_string())
                                                            .to_string(),
                                                    markup: None,
                                                },
                                            )
                                            .await?;
                                        return Ok(MessageAction::Handled);
                                    }
                                }
                            }

                            let buttons = provider_buttons();
                            adapter
                                .send_message(
                                    target,
                                    MessageContent {
                                        text: t!("domain.prov_title").to_string(),
                                        markup: Some(Markup { buttons }),
                                    },
                                )
                                .await?;
                            return Ok(MessageAction::Handled);
                        }
                        Err(e) => {
                            adapter
                                .send_message(
                                    target,
                                    MessageContent {
                                        text: e.to_string(),
                                        markup: None,
                                    },
                                )
                                .await?;
                            return Ok(MessageAction::Handled);
                        }
                    }
                }
                return Ok(MessageAction::Handled);
            }
            DomainInputStep::AwaitProvider => {
                if let Some(selection) = text {
                    let trimmed = selection.trim();
                    if let Some(provider) = parse_provider_selection(trimmed)
                        && state
                            .transition_domain_input(
                                target_str,
                                DomainInputStep::AwaitProvider,
                                DomainInputStep::AwaitCredentials(provider),
                                None,
                            )
                            .await
                    {
                        return Ok(MessageAction::Handled);
                    }
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("domain.prov_title").to_string(),
                                markup: Some(Markup {
                                    buttons: provider_buttons(),
                                }),
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }
                return Ok(MessageAction::Handled);
            }
            DomainInputStep::AwaitCredentials(selected_provider) => {
                if let Some(input) = text {
                    let trimmed = input.trim();
                    let parts: Vec<&str> = trimmed
                        .split([',', '，'])
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();

                    if parts.len() != 2 {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("domain.cred_invalid").to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                        return Ok(MessageAction::Handled);
                    }

                    let token = parts[0];
                    let key = parts[1];

                    if state
                        .transition_domain_input(
                            target_str,
                            DomainInputStep::AwaitCredentials(selected_provider),
                            DomainInputStep::Processing,
                            None,
                        )
                        .await
                    {
                        match AcmeManager::ensure_installed().await {
                            Ok(_) => {
                                let domain = domain_state
                                    .domain
                                    .as_ref()
                                    .expect("domain must be present for credential step");
                                match AcmeManager::issue_cert(
                                    domain,
                                    selected_provider,
                                    Some((token, key)),
                                )
                                .await
                                {
                                    Ok(cert_paths) => {
                                        state.take_domain_input(target_str).await;
                                        return Ok(MessageAction::DomainReady {
                                            source: domain_state.source,
                                            mode: XhttpDeployMode::Tls {
                                                domain: domain.clone(),
                                                cert_paths,
                                            },
                                        });
                                    }
                                    Err(e) => {
                                        state
                                            .transition_domain_input(
                                                target_str,
                                                DomainInputStep::Processing,
                                                DomainInputStep::AwaitCredentials(
                                                    selected_provider,
                                                ),
                                                None,
                                            )
                                            .await;
                                        adapter
                                            .send_message(
                                                target,
                                                MessageContent {
                                                    text:
                                                        t!("domain.cert_fail", "0" => e.to_string())
                                                            .to_string(),
                                                    markup: None,
                                                },
                                            )
                                            .await?;
                                        return Ok(MessageAction::Handled);
                                    }
                                }
                            }
                            Err(e) => {
                                state
                                    .transition_domain_input(
                                        target_str,
                                        DomainInputStep::Processing,
                                        DomainInputStep::AwaitCredentials(selected_provider),
                                        None,
                                    )
                                    .await;
                                adapter
                                    .send_message(
                                        target,
                                        MessageContent {
                                            text: t!("domain.install_fail", "0" => e.to_string())
                                                .to_string(),
                                            markup: None,
                                        },
                                    )
                                    .await?;
                                return Ok(MessageAction::Handled);
                            }
                        }
                    }
                }
                return Ok(MessageAction::Handled);
            }
            DomainInputStep::Processing => {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("domain.processing").to_string(),
                            markup: None,
                        },
                    )
                    .await?;
                return Ok(MessageAction::Handled);
            }
        }
    }

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

    Ok(MessageAction::NeedsDestruct)
}

fn provider_buttons() -> Vec<Vec<InlineButton>> {
    vec![
        vec![
            InlineButton {
                text: t!("domain.provider.cloudflare").to_string(),
                data: "prov:cloudflare".to_string(),
            },
            InlineButton {
                text: t!("domain.provider.aliyun").to_string(),
                data: "prov:aliyun".to_string(),
            },
        ],
        vec![
            InlineButton {
                text: t!("domain.provider.dnspod").to_string(),
                data: "prov:dnspod".to_string(),
            },
            InlineButton {
                text: t!("domain.provider.route53").to_string(),
                data: "prov:route53".to_string(),
            },
        ],
    ]
}

fn parse_provider_selection(text: &str) -> Option<DnsProvider> {
    match text.to_lowercase().as_str() {
        "cloudflare" | "cf" | "dns_cf" => Some(DnsProvider::Cloudflare),
        "aliyun" | "ali" | "dns_ali" => Some(DnsProvider::Aliyun),
        "dnspod" | "dp" | "dns_dp" => Some(DnsProvider::Dnspod),
        "route53" | "aws" | "dns_aws" => Some(DnsProvider::Route53),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::{
        BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
    };
    use crate::core::i18n;
    use crate::core::i18n::Lang;
    use crate::shared::types::TimeoutStatus;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct FakeState {
        source: DomainFlowSource,
        chat_id: String,
        inner: Arc<Mutex<FakeStateInner>>,
    }

    struct FakeStateInner {
        step: DomainInputStep,
        domain: Option<String>,
    }

    impl FakeState {
        fn domain(step: DomainInputStep) -> Self {
            Self {
                source: DomainFlowSource::Standalone,
                chat_id: "test_chat".to_string(),
                inner: Arc::new(Mutex::new(FakeStateInner {
                    step,
                    domain: Some("example.com".to_string()),
                })),
            }
        }

        fn credentials(provider: DnsProvider, domain: &str) -> Self {
            Self {
                source: DomainFlowSource::Standalone,
                chat_id: "test_chat".to_string(),
                inner: Arc::new(Mutex::new(FakeStateInner {
                    step: DomainInputStep::AwaitCredentials(provider),
                    domain: Some(domain.to_string()),
                })),
            }
        }

        fn snapshot(&self) -> DomainInputStep {
            self.inner.lock().unwrap().step.clone()
        }
    }

    #[async_trait]
    impl MessageState for FakeState {
        async fn schedule_timeout_status(
            &self,
            _chat_id: &str,
            _timeout: Duration,
        ) -> TimeoutStatus {
            TimeoutStatus::NotTracked
        }
        async fn remove_schedule_input(&self, _chat_id: &str) {}
        async fn take_warp_input_status(
            &self,
            _chat_id: &str,
            _timeout: Duration,
        ) -> TimeoutStatus {
            TimeoutStatus::NotTracked
        }
        async fn start_domain_input(
            &self,
            _chat_id: String,
            _source: DomainFlowSource,
            _now: std::time::Instant,
        ) {
        }
        async fn domain_input_snapshot(&self, chat_id: &str) -> Option<DomainInputState> {
            if chat_id == self.chat_id {
                let inner = self.inner.lock().unwrap();
                Some(DomainInputState {
                    updated_at: std::time::Instant::now(),
                    source: self.source,
                    step: inner.step.clone(),
                    domain: inner.domain.clone(),
                })
            } else {
                None
            }
        }
        async fn transition_domain_input(
            &self,
            _chat_id: &str,
            _expected: DomainInputStep,
            next: DomainInputStep,
            domain: Option<String>,
        ) -> bool {
            let mut inner = self.inner.lock().unwrap();
            inner.step = next;
            if domain.is_some() {
                inner.domain = domain;
            }
            true
        }
        async fn take_domain_input(&self, _chat_id: &str) -> Option<DomainInputState> {
            self.domain_input_snapshot(&self.chat_id).await
        }
        async fn domain_timeout_status(&self, _chat_id: &str, _timeout: Duration) -> TimeoutStatus {
            TimeoutStatus::Active
        }
    }

    struct RecordingAdapter {
        messages: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingAdapter {
        fn new() -> Self {
            Self {
                messages: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn last_text(&self) -> String {
            self.messages
                .lock()
                .unwrap()
                .last()
                .cloned()
                .unwrap_or_default()
        }
    }

    #[async_trait]
    impl BotAdapter for RecordingAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            content: MessageContent,
        ) -> Result<MessageId> {
            self.messages.lock().unwrap().push(content.text);
            Ok(MessageId("0".to_string()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> Result<()> {
            Ok(())
        }
        async fn delete_message(&self, _target: &TargetId, _msg_id: &MessageId) -> Result<()> {
            Ok(())
        }
        async fn answer_callback(
            &self,
            _target: &TargetId,
            _callback_id: &str,
            _text: Option<String>,
        ) -> Result<()> {
            Ok(())
        }
        async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities::TELEGRAM
        }
    }

    #[tokio::test]
    async fn empty_domain_keeps_await_domain_state() {
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::domain(DomainInputStep::AwaitDomain);

        let action = handle_message(&adapter, &target, Some("  "), false, &state)
            .await
            .unwrap();

        i18n::set_lang(Lang::En);
        assert!(matches!(action, MessageAction::Handled));
        assert!(matches!(state.snapshot(), DomainInputStep::AwaitDomain));
        assert_eq!(
            adapter.last_text(),
            "Domain cannot be empty, please re-enter."
        );
    }

    #[tokio::test]
    async fn credentials_require_exactly_two_nonempty_values() {
        i18n::set_lang(Lang::En);
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::credentials(DnsProvider::Cloudflare, "example.com");

        let action = handle_message(&adapter, &target, Some("one-value"), false, &state)
            .await
            .unwrap();

        assert!(matches!(action, MessageAction::Handled));
        assert!(matches!(
            state.snapshot(),
            DomainInputStep::AwaitCredentials(DnsProvider::Cloudflare)
        ));
        assert_eq!(
            adapter.last_text(),
            "Invalid credential format, please re-enter."
        );
    }
}
