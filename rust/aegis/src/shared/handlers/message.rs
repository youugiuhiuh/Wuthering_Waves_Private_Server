use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::app::workflows::schedule::ScheduleFlow;
use crate::app::workflows::warp::WarpFlow;
use crate::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::security::acme::{
    AcmeCertificateOperation, AcmeCommandError, AcmeFailureKind, AcmeManager, XhttpDeployMode,
};
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
    async fn schedule_flow(&self, chat_id: &str, timeout: Duration) -> ScheduleFlow;
    async fn warp_flow(&self, chat_id: &str, timeout: Duration) -> WarpFlow;
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

                            if let Some(provider) =
                                AcmeManager::configured_provider_for_domain(&domain)?
                                && state
                                    .transition_domain_input(
                                        target_str,
                                        DomainInputStep::AwaitDomain,
                                        DomainInputStep::Processing,
                                        Some(domain.clone()),
                                    )
                                    .await
                            {
                                let _install_result = AcmeManager::ensure_installed().await;
                                let operation = AcmeManager::operation_for_domain(&domain)?;
                                let msg = certificate_progress_message(&domain, operation);
                                adapter
                                    .send_message(
                                        target,
                                        MessageContent {
                                            text: msg,
                                            markup: None,
                                        },
                                    )
                                    .await?;
                                match AcmeManager::issue_cert_for_operation(
                                    &domain, provider, None, operation,
                                )
                                .await
                                {
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
                                                    text: t!("domain.cert_fail", "0" => localized_acme_failure(&e))
                                                        .to_string(),
                                                    markup: None,
                                                },
                                            )
                                            .await?;
                                        return Ok(MessageAction::Handled);
                                    }
                                }
                            }

                            return show_provider_selection(
                                adapter, target, state, target_str, domain,
                            )
                            .await;
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
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: provider_credential_guidance(provider),
                                    markup: None,
                                },
                            )
                            .await?;
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
                                                    text: t!("domain.cert_fail", "0" => localized_acme_failure(&e))
                                                        .to_string(),
                                                    markup: None,
                                                },
                                            )
                                            .await?;
                                        return Ok(MessageAction::Handled);
                                    }
                                }
                            }
                            Err(_) => {
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
                                            text: localized_acme_install_failure(),
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
        .schedule_flow(target_str, Duration::from_secs(180))
        .await
    {
        ScheduleFlow::Expired => {
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
        ScheduleFlow::Waiting => {
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
        ScheduleFlow::Continue => {}
    }

    // Warp input check
    match state.warp_flow(target_str, Duration::from_secs(60)).await {
        WarpFlow::Expired => {
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
        WarpFlow::Waiting => {
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
        WarpFlow::Continue => {}
    }

    Ok(MessageAction::NeedsDestruct)
}

async fn show_provider_selection(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    state: &dyn MessageState,
    target_str: &str,
    domain: String,
) -> anyhow::Result<MessageAction> {
    if !state
        .transition_domain_input(
            target_str,
            DomainInputStep::AwaitDomain,
            DomainInputStep::AwaitProvider,
            Some(domain),
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
    Ok(MessageAction::Handled)
}

fn provider_buttons() -> Vec<Vec<InlineButton>> {
    vec![vec![
        InlineButton {
            text: t!("domain.prov_cf").to_string(),
            data: "xhttp_domain_provider:cloudflare".to_string(),
        },
        InlineButton {
            text: t!("domain.prov_aws").to_string(),
            data: "xhttp_domain_provider:route53".to_string(),
        },
    ]]
}

pub(crate) fn provider_credential_guidance(provider: DnsProvider) -> String {
    let prompt = match provider {
        DnsProvider::Cloudflare => t!("domain.cred_prompt_cloudflare"),
        DnsProvider::Route53 => t!("domain.cred_prompt_route53"),
    };
    format!("{prompt}\n\n{}", t!("domain.cred_security_warning"))
}

fn localized_acme_failure(error: &anyhow::Error) -> String {
    match error
        .downcast_ref::<AcmeCommandError>()
        .map(|error| error.kind())
    {
        Some(AcmeFailureKind::Authentication) => t!("domain.acme_auth_error").to_string(),
        Some(AcmeFailureKind::Scope) => t!("domain.acme_scope_error").to_string(),
        Some(AcmeFailureKind::Dns) => t!("domain.acme_dns_error").to_string(),
        Some(AcmeFailureKind::Network) => t!("domain.acme_network_error").to_string(),
        Some(AcmeFailureKind::Timeout) => {
            format!("{} (ACME-TIMEOUT)", t!("domain.cert_timeout"))
        }
        Some(AcmeFailureKind::Unknown) | None => {
            t!("domain.acme_unknown_error", "0" => "ACME-UNKNOWN").to_string()
        }
    }
}

fn localized_acme_install_failure() -> String {
    t!("domain.acme_install_fail", "0" => "ACME-UNKNOWN").to_string()
}

fn certificate_progress_message(domain: &str, operation: AcmeCertificateOperation) -> String {
    match operation {
        AcmeCertificateOperation::Issue => t!("domain.issuing_cert", "0" => domain).to_string(),
        AcmeCertificateOperation::Renew => t!("domain.cert_renew").to_string(),
    }
}

fn parse_provider_selection(text: &str) -> Option<DnsProvider> {
    match text.to_lowercase().as_str() {
        "cloudflare" | "cf" | "dns_cf" => Some(DnsProvider::Cloudflare),
        "route53" | "aws" | "dns_aws" => Some(DnsProvider::Route53),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::i18n;
    use crate::core::i18n::Lang;
    use crate::shared::types::TimeoutStatus;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::BTreeMap;
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
        async fn schedule_flow(&self, _chat_id: &str, _timeout: Duration) -> ScheduleFlow {
            ScheduleFlow::Continue
        }
        async fn warp_flow(&self, _chat_id: &str, _timeout: Duration) -> WarpFlow {
            WarpFlow::Continue
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
            chat_id: &str,
            expected: DomainInputStep,
            next: DomainInputStep,
            domain: Option<String>,
        ) -> bool {
            if chat_id != self.chat_id {
                return false;
            }
            let mut inner = self.inner.lock().unwrap();
            if inner.step != expected {
                return false;
            }
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
        button_data: Arc<Mutex<Vec<String>>>,
        button_text: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingAdapter {
        fn new() -> Self {
            Self {
                messages: Arc::new(Mutex::new(Vec::new())),
                button_data: Arc::new(Mutex::new(Vec::new())),
                button_text: Arc::new(Mutex::new(Vec::new())),
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
            if let Some(markup) = &content.markup {
                for row in &markup.buttons {
                    for button in row {
                        self.button_data.lock().unwrap().push(button.data.clone());
                        self.button_text.lock().unwrap().push(button.text.clone());
                    }
                }
            }
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
    }

    #[test]
    fn provider_guidance_runtime_never_returns_raw_keys() {
        for provider in [DnsProvider::Cloudflare, DnsProvider::Route53] {
            let text = provider_credential_guidance(provider);
            assert!(!text.contains("domain.cred_prompt_"));
            assert!(!text.contains("domain.cred_security_warning"));
        }
    }

    #[test]
    fn acme_failures_render_safe_localized_guidance() {
        i18n::set_lang(Lang::En);
        let cases = [
            (AcmeFailureKind::Authentication, "ACME-AUTH"),
            (AcmeFailureKind::Scope, "ACME-SCOPE"),
            (AcmeFailureKind::Dns, "ACME-DNS"),
            (AcmeFailureKind::Network, "ACME-NETWORK"),
            (AcmeFailureKind::Timeout, "ACME-TIMEOUT"),
            (AcmeFailureKind::Unknown, "ACME-UNKNOWN"),
        ];

        for (kind, code) in cases {
            let error = anyhow::Error::new(AcmeCommandError::new(kind))
                .context("raw provider detail must stay hidden");
            let rendered = localized_acme_failure(&error);

            assert!(rendered.contains(code), "missing {code}: {rendered}");
            assert!(!rendered.contains("domain.acme_"));
            assert!(!rendered.contains("domain.cert_timeout"));
            assert!(!rendered.contains("raw provider detail"));
        }

        let rendered = localized_acme_failure(&anyhow::anyhow!(
            "untyped subprocess output must stay hidden"
        ));
        assert!(rendered.contains("ACME-UNKNOWN"));
        assert!(!rendered.contains("domain.acme_unknown_error"));
        assert!(!rendered.contains("untyped subprocess output"));
    }

    #[test]
    fn acme_install_failure_hides_arbitrary_detail() {
        i18n::set_lang(Lang::En);

        let rendered = localized_acme_install_failure();

        assert!(rendered.contains("ACME-UNKNOWN"));
        assert!(!rendered.contains("domain.acme_install_fail"));
    }

    #[test]
    fn new_issuance_uses_existing_issuance_message() {
        assert_eq!(
            certificate_progress_message("example.com", AcmeCertificateOperation::Issue),
            t!("domain.issuing_cert", "0" => "example.com").to_string()
        );
    }

    #[test]
    fn renewal_uses_existing_renewal_message() {
        assert_eq!(
            certificate_progress_message("example.com", AcmeCertificateOperation::Renew),
            t!("domain.cert_renew").to_string()
        );
    }

    #[test]
    fn domain_translation_keys_exist() {
        fn domain_entries(yaml: &str) -> BTreeMap<&str, &str> {
            yaml.split_once("\ndomain:\n")
                .expect("domain section")
                .1
                .lines()
                .take_while(|line| line.starts_with("  ") || line.is_empty())
                .filter_map(|line| line.trim().split_once(": "))
                .collect()
        }

        let locales = [
            domain_entries(include_str!("../../resources/i18n/zh.yml")),
            domain_entries(include_str!("../../resources/i18n/en.yml")),
            domain_entries(include_str!("../../resources/i18n/ja.yml")),
        ];
        let required = [
            "cred_prompt_cloudflare",
            "cred_prompt_route53",
            "cred_security_warning",
            "acme_auth_error",
            "acme_scope_error",
            "acme_dns_error",
            "acme_network_error",
            "acme_unknown_error",
        ];

        assert_eq!(
            locales[0].keys().collect::<Vec<_>>(),
            locales[1].keys().collect::<Vec<_>>()
        );
        assert_eq!(
            locales[1].keys().collect::<Vec<_>>(),
            locales[2].keys().collect::<Vec<_>>()
        );
        let providers = [
            (
                "cred_prompt_cloudflare",
                "API_TOKEN,ZONE_ID",
                "https://dash.cloudflare.com/profile/api-tokens",
                &["Zone > DNS > Edit", "Zone > Zone > Read", "Zone ID"][..],
            ),
            (
                "cred_prompt_route53",
                "ACCESS_KEY_ID,SECRET_ACCESS_KEY",
                "https://console.aws.amazon.com/iam/home#/users",
                &[
                    "route53:ListHostedZones",
                    "route53:ListResourceRecordSets",
                    "route53:ChangeResourceRecordSets",
                ][..],
            ),
        ];

        let network_requirements = [
            &["订单状态", "速率限制", "等待", "重试", "连接"][..],
            &[
                "order status",
                "rate limit",
                "wait",
                "retry",
                "connectivity",
            ][..],
            &["注文ステータス", "レート制限", "待って", "再試行", "接続"][..],
        ];
        let cloudflare_scope_and_location = [
            &["资源范围限制到该区域", "域名概述页", "API 区域"][..],
            &[
                "resources limited to the target zone",
                "domain Overview page",
                "API section",
            ][..],
            &[
                "リソース範囲を対象ゾーンに限定",
                "ドメイン概要ページ",
                "API セクション",
            ][..],
        ];

        for ((locale, network_requirements), cloudflare_requirements) in locales
            .into_iter()
            .zip(network_requirements)
            .zip(cloudflare_scope_and_location)
        {
            for key in required {
                assert!(locale.contains_key(key), "missing domain.{key}");
            }
            assert!(locale["acme_unknown_error"].contains("%{0}"));
            for requirement in cloudflare_requirements {
                assert!(
                    locale["cred_prompt_cloudflare"].contains(requirement),
                    "domain.cred_prompt_cloudflare missing {requirement}"
                );
            }
            for requirement in network_requirements {
                assert!(
                    locale["acme_network_error"].contains(requirement),
                    "domain.acme_network_error missing {requirement}"
                );
            }
            for (key, fields, url, permissions) in providers {
                let text = locale[key];
                assert!(text.contains(fields), "domain.{key} missing {fields}");
                assert!(text.contains(url), "domain.{key} missing {url}");
                if key == "cred_prompt_cloudflare" {
                    assert!(!text.contains("ACCOUNT_ID"));
                }
                for permission in permissions {
                    assert!(
                        text.contains(permission),
                        "domain.{key} missing {permission}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn typed_provider_selection_sends_provider_guidance() {
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::domain(DomainInputStep::AwaitProvider);

        handle_message(&adapter, &target, Some("cloudflare"), false, &state)
            .await
            .unwrap();

        assert!(matches!(
            state.snapshot(),
            DomainInputStep::AwaitCredentials(DnsProvider::Cloudflare)
        ));
        assert!(!adapter.last_text().contains("domain.cred_prompt_"));
        assert!(!adapter.last_text().contains("domain.cred_security_warning"));
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

    #[tokio::test]
    async fn provider_fallback_presents_routable_buttons() {
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::domain(DomainInputStep::AwaitDomain);

        let action = show_provider_selection(
            &adapter,
            &target,
            &state,
            &target.0,
            "no-certificate.invalid".to_string(),
        )
        .await
        .unwrap();

        assert!(matches!(action, MessageAction::Handled));
        assert!(matches!(state.snapshot(), DomainInputStep::AwaitProvider));
        assert_ne!(adapter.last_text(), "domain.prov_title");
        assert_eq!(
            *adapter.button_data.lock().unwrap(),
            vec![
                "xhttp_domain_provider:cloudflare".to_string(),
                "xhttp_domain_provider:route53".to_string(),
            ]
        );
        let button_text = adapter.button_text.lock().unwrap();
        for raw_key in ["domain.prov_cf", "domain.prov_aws"] {
            assert!(!button_text.contains(&raw_key.to_string()));
        }
    }

    #[tokio::test]
    async fn fake_domain_transition_is_compare_and_set() {
        let state = FakeState::domain(DomainInputStep::AwaitProvider);

        assert!(
            !state
                .transition_domain_input(
                    "test_chat",
                    DomainInputStep::AwaitDomain,
                    DomainInputStep::Processing,
                    None,
                )
                .await
        );
        assert!(matches!(state.snapshot(), DomainInputStep::AwaitProvider));
    }
}
