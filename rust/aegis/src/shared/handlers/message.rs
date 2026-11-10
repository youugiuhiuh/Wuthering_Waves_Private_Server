use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::app::state::{XdriveWizardSession, XdriveWizardStep};
use crate::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::security::acme::{
    AcmeCertificateOperation, AcmeCommandError, AcmeFailureKind, AcmeManager, XhttpDeployMode,
};
use crate::core::types::{DnsProvider, DomainFlowSource, DomainInputState, DomainInputStep};
use crate::core::xray::config::ConfigManager;
use crate::core::xray::routing::{
    CustomAddOutcome, RoutingManager, match_custom_direct, matches_builtin_direct,
};
use crate::core::xray::xdrive::{
    ONEDRIVE_DEFAULT_CLIENT_ID, ONEDRIVE_DEFAULT_CLIENT_SECRET, XdriveConfig,
};
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
    async fn get_xdrive_wizard(&self, target: &str) -> Option<XdriveWizardSession>;
    async fn update_xdrive_wizard(&self, target: &str, session: XdriveWizardSession);
    async fn clear_xdrive_wizard(&self, target: &str);
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

    if let Some(session) = state.get_xdrive_wizard(target_str).await {
        return handle_xdrive_wizard_input(adapter, target, text, state, session).await;
    }

    if let Some(domain_state) = state.domain_input_snapshot(target_str).await {
        // 自定义放行（添加 / 自检）与 ACME 共用 AwaitDomain 这一步输入，但语义完全不同：
        // ACME 分支会把输入当作「要签发证书的域名」。因此必须【先按来源分流】，
        // 否则用户在「添加放行域名」里输入 decodo.cn 会被拿去申请证书（可能真的签发）。
        // 这里显式列出 ACME 的两个来源（而不是用 `_`）是为了：日后新增来源时必须回来
        // 决定它走写盘还是只读分支，编译器会把这件事顶到眼前。
        match domain_state.source {
            DomainFlowSource::CustomAllowlist => {
                return handle_custom_allowlist_input(adapter, target, text, state, target_str)
                    .await;
            }
            DomainFlowSource::CustomAllowlistCheck => {
                return handle_custom_allowlist_check_input(
                    adapter, target, text, state, target_str,
                )
                .await;
            }
            DomainFlowSource::Standalone | DomainFlowSource::OneClick => {}
        }

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

/// 判定当前输入状态是否属于「自定义放行」流程。
///
/// 返回 `None` 表示「不属于该来源」：调用方据此【完全不进】自定义放行分支，
/// 反过来 `handle_message` 也只在来源匹配时才进这里。为什么要把来源判断做成返回值
/// 而不是布尔：调用方拿不到 `Some` 就无法进入写盘路径，「不干扰 ACME」是类型层面的。
/// 添加与自检共用同一套归一化：两者对「什么算合法域名」必须给出完全一致的口径，
/// 否则会出现「添加时被拒、自检却接受」这种自相矛盾的回答。
/// 输入先 `trim` 再交给 `normalize_custom_domain`，后者会自己再做一次 trim，
/// 这里显式 trim 是为了让「空输入」与「带空格的输入」走向同一个判定结果。
pub(crate) fn custom_allowlist_decision(
    source: &crate::core::types::DomainFlowSource,
    text: &str,
) -> Option<Result<String, crate::core::xray::routing::CustomDomainError>> {
    if !matches!(
        source,
        DomainFlowSource::CustomAllowlist | DomainFlowSource::CustomAllowlistCheck
    ) {
        return None;
    }
    Some(crate::core::xray::routing::normalize_custom_domain(
        text.trim(),
    ))
}

/// 把校验错误映射到 i18n key（UI 文案选择用）。
///
/// 只有三类拒绝原因各有专属文案：用户看到后能直接知道该改什么。
/// 其余原因（空/带端口/非法 label/超长/未知前缀/非 ASCII）对用户而言动作相同
/// ——「重新输入一个合法域名」，共用一条文案以免三语维护成本翻倍。
pub(crate) fn custom_domain_error_key(
    e: &crate::core::xray::routing::CustomDomainError,
) -> &'static str {
    use crate::core::xray::routing::CustomDomainError as E;

    match e {
        E::HasSchemeOrPath => "xray.routing_custom_invalid_scheme",
        E::SingleLabel => "xray.routing_custom_invalid_single_label",
        E::IpNotSupported => "xray.routing_custom_invalid_ip",
        E::Empty
        | E::HasPort
        | E::InvalidLabel
        | E::TooLong
        | E::UnsupportedPrefix
        | E::NonAscii => "xray.routing_custom_invalid_generic",
    }
}

/// 自定义放行（添加 / 自检）共用的输入前处理：超时、非文本、空输入。
///
/// 两种流程共用同一份引导文案（`xray.routing_custom_input_prompt`），因此这三种
/// 「还没到判定阶段」的反应必须逐字一致；抽出来是为了以后加第三种流程时无法只改一半。
/// 返回 `Some(input)` 表示可以进入判定；`None` 表示本次消息已回应完毕（调用方直接返回 Handled）。
async fn take_custom_input<'a>(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: Option<&'a str>,
    state: &dyn MessageState,
    target_str: &str,
) -> anyhow::Result<Option<&'a str>> {
    // 120 秒沿用 ACME 的口径：引导文案（xray.routing_custom_input_prompt）已向用户承诺 120 秒，
    // 两边用同一个数字才能保证「文案说多久就多久」
    match state
        .domain_timeout_status(target_str, Duration::from_secs(120))
        .await
    {
        TimeoutStatus::Expired => {
            state.take_domain_input(target_str).await;
            send_plain(adapter, target, t!("domain.input_timeout").to_string()).await?;
            return Ok(None);
        }
        TimeoutStatus::Active | TimeoutStatus::NotTracked => {}
    }

    // 非文本（例如用户回了张图）不消费这条消息，保持等待即可
    let Some(input) = text else {
        return Ok(None);
    };

    if input.trim().is_empty() {
        send_plain(adapter, target, t!("domain.input_empty").to_string()).await?;
        return Ok(None);
    }

    Ok(Some(input))
}

async fn complete_xdrive_node(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    state: &dyn MessageState,
    session: &XdriveWizardSession,
    xdrive_config: &XdriveConfig,
    cancel_markup: Option<Markup>,
) -> anyhow::Result<MessageAction> {
    if let Err(e) = xdrive_config.validate() {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: format!("❌ 配置校验失败: {}\n\n请点击下方按钮重新配置。", e),
                    markup: cancel_markup,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    state.clear_xdrive_wizard(&target.0).await;
    let tag = format!("XDRIVE-{}", session.region.to_ascii_uppercase());
    match ConfigManager::create_xdrive_node(xdrive_config, &tag).await {
        Ok(res) => {
            let link = res.links.first().cloned().unwrap_or_default();
            if !link.is_empty() {
                // 1. 发送单独纯链接消息，客户端/手机点击即可一键复制或导入
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: link,
                            markup: None,
                        },
                    )
                    .await?;
            }
            // 2. 发送创建成功摘要说明
            let msg = t!("xray.xdrive_success_summary", "0" => tag);
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: msg.to_string(),
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
                        text: format!("❌ 节点生成失败: {}", e),
                        markup: None,
                    },
                )
                .await?;
        }
    }
    Ok(MessageAction::Handled)
}

/// 智能提取 Microsoft OneDrive 凭据（支持 rclone authorize "onedrive" JSON、URL 或 Key-Value 多行文本）
/// 返回 (client_id, client_secret, refresh_token, folder_id)
pub fn extract_onedrive_tokens(text: &str) -> Option<(String, String, String, Option<String>)> {
    let mut client_id = None;
    let mut client_secret = None;
    let mut refresh_token = None;
    let mut folder_id = None;

    let clean_val = |raw: &str| -> String {
        raw.trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ';')
            .trim()
            .to_string()
    };

    let parse_json = |raw: &str| -> Option<serde_json::Value> {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
            return Some(v);
        }
        if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}'))
            && start < end
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw[start..=end])
        {
            return Some(v);
        }
        None
    };

    // 1. JSON 格式支持（支持原生 JSON 或包含在 rclone 命令输出中的 JSON 块）
    if let Some(v) = parse_json(text) {
        if let Some(cid) = v
            .get("client_id")
            .or_else(|| v.get("clientId"))
            .or_else(|| v.get("appId"))
            .or_else(|| v.get("app_id"))
            .and_then(|s| s.as_str())
        {
            let s = clean_val(cid);
            if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                client_id = Some(s);
            }
        }
        if let Some(cs) = v
            .get("client_secret")
            .or_else(|| v.get("clientSecret"))
            .or_else(|| v.get("secret"))
            .and_then(|s| s.as_str())
        {
            let s = clean_val(cs);
            if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                client_secret = Some(s);
            }
        }
        if let Some(rt) = v
            .get("refresh_token")
            .or_else(|| v.get("refreshToken"))
            .and_then(|s| s.as_str())
        {
            let s = clean_val(rt);
            if !s.is_empty() {
                refresh_token = Some(s);
            }
        } else if let Some(token_val) = v.get("token") {
            if let Some(rt) = token_val
                .get("refresh_token")
                .or_else(|| token_val.get("refreshToken"))
                .and_then(|s| s.as_str())
            {
                let s = clean_val(rt);
                if !s.is_empty() {
                    refresh_token = Some(s);
                }
            } else if let Some(token_str) = token_val.as_str()
                && let Some(tv) = parse_json(token_str)
                && let Some(rt) = tv
                    .get("refresh_token")
                    .or_else(|| tv.get("refreshToken"))
                    .and_then(|s| s.as_str())
            {
                let s = clean_val(rt);
                if !s.is_empty() {
                    refresh_token = Some(s);
                }
            }
        }
        if let Some(fid) = v
            .get("folder")
            .or_else(|| v.get("folder_id"))
            .or_else(|| v.get("remote_folder"))
            .and_then(|s| s.as_str())
        {
            let s = clean_val(fid);
            if !s.is_empty() {
                folder_id = Some(s);
            }
        }
    }

    // 2. 逐行分析
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();

        let (key_part, val_part) = if let Some(pos) = line.find(['=', ':']) {
            let k = line[..pos].trim().to_ascii_lowercase();
            let v = line[pos + 1..].trim();
            (Some(k), Some(v))
        } else {
            (None, None)
        };

        // Client ID 提取
        if client_id.is_none() {
            if let (Some(k), Some(v)) = (&key_part, val_part)
                && (k.contains("client_id")
                    || k.contains("client id")
                    || k.contains("clientid")
                    || k.contains("app_id")
                    || k.contains("appid")
                    || k.contains("客户端 id")
                    || k.contains("客户端id")
                    || k.contains("应用程序 id")
                    || k.contains("应用程序id"))
                && !v.is_empty()
            {
                let s = clean_val(v);
                if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                    client_id = Some(s);
                }
            }
            if client_id.is_none()
                && (lower == "client id"
                    || lower == "client_id"
                    || lower == "app id"
                    || lower == "appid"
                    || lower == "客户端 id"
                    || lower == "应用程序 id")
                && let Some(next) = lines.get(i + 1)
            {
                let s = clean_val(next);
                if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                    client_id = Some(s);
                }
            }
        }

        // Client Secret 提取
        if client_secret.is_none() {
            if let (Some(k), Some(v)) = (&key_part, val_part)
                && (k.contains("client_secret")
                    || k.contains("client secret")
                    || k.contains("clientsecret")
                    || k.contains("客户端密码")
                    || k.contains("客户端密钥")
                    || k.contains("客户端机密")
                    || k == "secret")
                && !v.is_empty()
            {
                let s = clean_val(v);
                if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                    client_secret = Some(s);
                }
            }
            if client_secret.is_none()
                && (lower == "client secret"
                    || lower == "client_secret"
                    || lower == "客户端密码"
                    || lower == "客户端密钥"
                    || lower == "secret")
                && let Some(next) = lines.get(i + 1)
            {
                let s = clean_val(next);
                if !s.is_empty() && s != "-" && !s.eq_ignore_ascii_case("none") {
                    client_secret = Some(s);
                }
            }
        }

        // Refresh Token 提取
        if refresh_token.is_none() {
            if let (Some(k), Some(v)) = (&key_part, val_part)
                && (k.contains("refresh_token")
                    || k.contains("refresh token")
                    || k.contains("refreshtoken")
                    || k.contains("刷新令牌"))
                && !v.is_empty()
            {
                let s = clean_val(v);
                if !s.is_empty() {
                    refresh_token = Some(s);
                }
            }
            if refresh_token.is_none()
                && (lower == "refresh token" || lower == "refresh_token" || lower == "刷新令牌")
                && let Some(next) = lines.get(i + 1)
            {
                let s = clean_val(next);
                if !s.is_empty() {
                    refresh_token = Some(s);
                }
            }
        }

        // Folder 提取
        if folder_id.is_none()
            && let (Some(k), Some(v)) = (&key_part, val_part)
            && (k.contains("folder")
                || k.contains("remote_folder")
                || k.contains("中转文件夹")
                || k.contains("文件夹"))
            && !v.is_empty()
        {
            let s = clean_val(v);
            if !s.is_empty() {
                folder_id = Some(s);
            }
        }
    }

    if let Some(rt) = refresh_token
        && !rt.trim().is_empty()
    {
        let final_cid = match client_id {
            Some(cid)
                if !cid.trim().is_empty() && cid != "-" && !cid.eq_ignore_ascii_case("none") =>
            {
                cid
            }
            _ => ONEDRIVE_DEFAULT_CLIENT_ID.to_string(),
        };
        let final_sec = match client_secret {
            Some(sec)
                if !sec.trim().is_empty() && sec != "-" && !sec.eq_ignore_ascii_case("none") =>
            {
                sec
            }
            _ => ONEDRIVE_DEFAULT_CLIENT_SECRET.to_string(),
        };
        Some((
            final_cid,
            final_sec,
            rt,
            folder_id.filter(|f| !f.trim().is_empty()),
        ))
    } else {
        None
    }
}

async fn handle_xdrive_wizard_input(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: Option<&str>,
    state: &dyn MessageState,
    mut session: XdriveWizardSession,
) -> anyhow::Result<MessageAction> {
    let raw_input = match text {
        Some(t) => t.trim(),
        None => return Ok(MessageAction::Handled),
    };

    if raw_input == "/cancel" {
        state.clear_xdrive_wizard(&target.0).await;
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("xray.xdrive_cancelled").to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    if raw_input.is_empty() {
        return Ok(MessageAction::Handled);
    }

    let cancel_markup = Some(Markup {
        buttons: vec![vec![InlineButton {
            text: t!("xray.xdrive_cancel_btn").into(),
            data: "u_xdrive_cancel".into(),
        }]],
    });

    // 如果是 CN / R2 模式，且输入内容包含 r2.cloudflarestorage.com，尝试一键提取全文凭据
    if (session.region == "cn" || session.region == "r2")
        && raw_input.contains("r2.cloudflarestorage.com")
        && let Some((acc, ak, sk)) = XdriveConfig::extract_cloudflare_r2_page_tokens(raw_input)
    {
        session.account_id = Some(acc.clone());
        session.endpoint = None;
        session.access_key = Some(ak.clone());
        session.secret_key = Some(sk.clone());
        session.step = XdriveWizardStep::Bucket;
        state.update_xdrive_wizard(&target.0, session).await;
        let masked_sk = if sk.len() > 16 {
            format!("{}...", &sk[..16])
        } else {
            "***".to_string()
        };
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!(
                        "xray.xdrive_r2_auto_detected",
                        "0" => acc,
                        "1" => ak,
                        "2" => masked_sk
                    )
                    .to_string(),
                    markup: cancel_markup,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    // Microsoft OneDrive 全文凭据智能提取（支持 rclone authorize "onedrive" JSON、URL 或多行文本）
    let raw_lower = raw_input.to_ascii_lowercase();
    let is_onedrive_explicit = raw_lower.contains("login.microsoftonline.com")
        || raw_lower.contains("graph.microsoft.com")
        || raw_lower.contains("onedrive")
        || (raw_lower.contains("rclone") && raw_lower.contains("authorize"))
        || ((raw_lower.contains("microsoft")
            || raw_lower.contains("azure")
            || raw_lower.contains("entra"))
            && raw_lower.contains("refresh_token"));
    let is_onedrive_session = session.region == "onedrive"
        && (raw_lower.contains("refresh_token")
            || raw_lower.contains("refreshtoken")
            || raw_lower.contains("access_token")
            || raw_lower.contains("刷新令牌")
            || raw_lower.starts_with('{'));
    let is_onedrive_tokens = is_onedrive_explicit || is_onedrive_session;

    if is_onedrive_tokens
        && let Some((cid, sec, ref_tok, opt_folder)) = extract_onedrive_tokens(raw_input)
    {
        session.region = "onedrive".to_string();
        session.client_id = Some(cid.clone());
        session.client_secret = Some(sec.clone());
        session.refresh_token = Some(ref_tok.clone());
        if let Some(folder) = opt_folder.or_else(|| session.folder_id.clone()) {
            session.folder_id = Some(folder.clone());
            let cfg = if cid == ONEDRIVE_DEFAULT_CLIENT_ID && sec == ONEDRIVE_DEFAULT_CLIENT_SECRET
            {
                XdriveConfig::new_onedrive(&ref_tok, &folder)
            } else {
                XdriveConfig::new_onedrive_custom(&cid, &sec, &ref_tok, &folder)
            };
            return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                .await;
        } else {
            session.step = XdriveWizardStep::FolderId;
            state.update_xdrive_wizard(&target.0, session).await;

            let masked_sk = if sec.is_empty() {
                "-".to_string()
            } else if sec.len() > 8 {
                format!("{}...", &sec[..8])
            } else {
                "***".to_string()
            };
            let masked_rf = if ref_tok.len() > 12 {
                format!("{}...", &ref_tok[..12])
            } else {
                "***".to_string()
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!(
                            "xray.xdrive_onedrive_auto_detected",
                            "0" => cid,
                            "1" => masked_sk,
                            "2" => masked_rf
                        )
                        .to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
    }

    // Google Drive 全文凭据智能提取（支持 playground URL、JSON 或 client_id + refresh_token 文本）
    let is_gdrive_tokens = !is_onedrive_tokens
        && (session.region != "onedrive")
        && (raw_lower.contains("developers.google.com/oauthplayground")
            || (raw_lower.contains("client_id") && raw_lower.contains("refresh_token")));

    if is_gdrive_tokens
        && let Some((cid, sec, ref_tok, opt_folder)) =
            XdriveConfig::extract_google_drive_tokens(raw_input)
    {
        session.region = "gdrive".to_string();
        session.client_id = Some(cid.clone());
        session.client_secret = Some(sec.clone());
        session.refresh_token = Some(ref_tok.clone());
        if let Some(folder) = opt_folder {
            session.folder_id = Some(folder);
        }
        session.step = XdriveWizardStep::FolderId;
        state.update_xdrive_wizard(&target.0, session).await;

        let masked_sk = if sec.len() > 8 {
            format!("{}...", &sec[..8])
        } else {
            "***".to_string()
        };
        let masked_rf = if ref_tok.len() > 12 {
            format!("{}...", &ref_tok[..12])
        } else {
            "***".to_string()
        };
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!(
                        "xray.xdrive_gdrive_auto_detected",
                        "0" => cid,
                        "1" => masked_sk,
                        "2" => masked_rf
                    )
                    .to_string(),
                    markup: cancel_markup,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    // AWS S3 全文凭据智能提取（支持 INI credentials、CSV、JSON、URL 等）
    if session.region == "aws"
        && let Some((reg, ak, sk, opt_bucket)) = XdriveConfig::extract_aws_s3_tokens(raw_input)
    {
        session.s3_region = Some(reg.clone());
        session.access_key = Some(ak.clone());
        session.secret_key = Some(sk.clone());
        if let Some(bucket) = opt_bucket {
            session.bucket = Some(bucket.clone());
            let cfg = XdriveConfig::new_aws_s3(&reg, &bucket, &ak, &sk, "aegis-xdrive");
            return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                .await;
        } else {
            session.step = XdriveWizardStep::Bucket;
            state.update_xdrive_wizard(&target.0, session).await;
            let masked_sk = if sk.len() > 16 {
                format!("{}...", &sk[..16])
            } else {
                "***".to_string()
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!(
                            "xray.xdrive_aws_auto_detected",
                            "0" => reg,
                            "1" => ak,
                            "2" => masked_sk
                        )
                        .to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
    }

    match session.step {
        XdriveWizardStep::AccountId => {
            if session.region == "oracle" {
                session.account_id = Some(raw_input.to_string());
                session.step = XdriveWizardStep::Region;
                state.update_xdrive_wizard(&target.0, session).await;
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("xray.xdrive_guide_oracle_step2").to_string(),
                            markup: cancel_markup,
                        },
                    )
                    .await?;
                return Ok(MessageAction::Handled);
            }

            if raw_input.starts_with("cfat_") {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("xray.xdrive_cfat_warning").to_string(),
                            markup: cancel_markup,
                        },
                    )
                    .await?;
                return Ok(MessageAction::Handled);
            }

            let extracted = match XdriveConfig::extract_cloudflare_account_id(raw_input) {
                Some(id) => id,
                None => {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("xray.xdrive_invalid_account_id").to_string(),
                                markup: cancel_markup,
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }
            };

            if extracted.len() == 32 && extracted.chars().all(|c| c.is_ascii_hexdigit()) {
                session.account_id = Some(extracted);
                session.endpoint = None;
            } else {
                session.endpoint = Some(extracted);
                session.account_id = None;
            }
            session.step = XdriveWizardStep::Bucket;
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("xray.xdrive_guide_cn_step2").to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::Endpoint => {
            session.endpoint = Some(raw_input.to_string());
            session.step = XdriveWizardStep::Region;
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("xray.xdrive_guide_custom_step2").to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::Region => {
            session.s3_region = Some(raw_input.to_string());
            session.step = XdriveWizardStep::Bucket;
            let prompt = if session.region == "aws" {
                t!("xray.xdrive_guide_aws_step2")
            } else if session.region == "oracle" {
                t!("xray.xdrive_guide_oracle_step3")
            } else if session.region == "b2" {
                t!("xray.xdrive_guide_b2_step2")
            } else {
                t!("xray.xdrive_guide_custom_step3")
            };
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: prompt.to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::Bucket => {
            session.bucket = Some(raw_input.to_string());

            // 如果此前已成功自动提取了凭据（AWS 模式下等待输入 Bucket）
            if session.region == "aws"
                && let (Some(reg), Some(ak), Some(sk)) = (
                    session.s3_region.as_deref(),
                    session.access_key.as_deref(),
                    session.secret_key.as_deref(),
                )
            {
                let cfg = XdriveConfig::new_aws_s3(reg, raw_input, ak, sk, "aegis-xdrive");
                return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                    .await;
            }

            if session.region == "oracle"
                && let (Some(ns), Some(reg), Some(ak), Some(sk)) = (
                    session.account_id.as_deref(),
                    session.s3_region.as_deref(),
                    session.access_key.as_deref(),
                    session.secret_key.as_deref(),
                )
            {
                let cfg =
                    XdriveConfig::new_oracle_cloud_s3(ns, reg, raw_input, ak, sk, "aegis-xdrive");
                return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                    .await;
            }

            if session.region == "b2"
                && let (Some(reg), Some(ak), Some(sk)) = (
                    session.s3_region.as_deref(),
                    session.access_key.as_deref(),
                    session.secret_key.as_deref(),
                )
            {
                let cfg = XdriveConfig::new_backblaze_b2(reg, raw_input, ak, sk, "aegis-xdrive");
                return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                    .await;
            }

            session.step = XdriveWizardStep::AccessKey;
            let prompt = match session.region.as_str() {
                "aws" => t!("xray.xdrive_guide_aws_step3"),
                "oracle" => t!("xray.xdrive_guide_oracle_step4"),
                "b2" => t!("xray.xdrive_guide_b2_step3"),
                "cn" | "r2" => t!("xray.xdrive_guide_cn_step3"),
                "ru" | "yandex" => t!("xray.xdrive_guide_ru_step2"),
                "ir" | "arvan" => t!("xray.xdrive_guide_ir_step2"),
                _ => t!("xray.xdrive_guide_custom_step4"),
            };
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: prompt.to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::AccessKey => {
            if (session.region == "cn" || session.region == "r2") && raw_input.starts_with("cfat_")
            {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("xray.xdrive_cfat_warning").to_string(),
                            markup: cancel_markup,
                        },
                    )
                    .await?;
                return Ok(MessageAction::Handled);
            }

            session.access_key = Some(raw_input.to_string());
            session.step = XdriveWizardStep::SecretKey;
            let prompt = match session.region.as_str() {
                "aws" => t!("xray.xdrive_guide_aws_step4"),
                "oracle" => t!("xray.xdrive_guide_oracle_step5"),
                "b2" => t!("xray.xdrive_guide_b2_step4"),
                "cn" | "r2" => t!("xray.xdrive_guide_cn_step4"),
                "ru" | "yandex" => t!("xray.xdrive_guide_ru_step3"),
                "ir" | "arvan" => t!("xray.xdrive_guide_ir_step3"),
                _ => t!("xray.xdrive_guide_custom_step5"),
            };
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: prompt.to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::SecretKey => {
            session.secret_key = Some(raw_input.to_string());

            let cfg = match session.region.as_str() {
                "aws" => {
                    let region = session.s3_region.as_deref().unwrap_or("us-east-1");
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_aws_s3(region, bucket, ak, sk, "aegis-xdrive")
                }
                "oracle" => {
                    let ns = session.account_id.as_deref().unwrap_or_default();
                    let region = session.s3_region.as_deref().unwrap_or("ap-chuncheon-1");
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_oracle_cloud_s3(ns, region, bucket, ak, sk, "aegis-xdrive")
                }
                "b2" => {
                    let region = session.s3_region.as_deref().unwrap_or("us-west-004");
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_backblaze_b2(region, bucket, ak, sk, "aegis-xdrive")
                }
                "cn" | "r2" => {
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    if let Some(endpoint) = session.endpoint.as_deref() {
                        XdriveConfig::new_cloudflare_r2_with_endpoint(
                            endpoint,
                            bucket,
                            ak,
                            sk,
                            "aegis-xdrive",
                        )
                    } else {
                        let account_id = session.account_id.as_deref().unwrap_or_default();
                        XdriveConfig::new_cloudflare_r2(account_id, bucket, ak, sk, "aegis-xdrive")
                    }
                }
                "ru" | "yandex" => {
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_s3(
                        "https://storage.yandexcloud.net",
                        "ru-central1",
                        bucket,
                        ak,
                        sk,
                        true,
                        "aegis-xdrive",
                    )
                }
                "ir" | "arvan" => {
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_s3(
                        "https://s3.ir-thr-at1.arvanstorage.ir",
                        "ir-thr-at1",
                        bucket,
                        ak,
                        sk,
                        true,
                        "aegis-xdrive",
                    )
                }
                _ => {
                    let endpoint = session.endpoint.as_deref().unwrap_or_default();
                    let region = session.s3_region.as_deref().unwrap_or("us-east-1");
                    let bucket = session.bucket.as_deref().unwrap_or_default();
                    let ak = session.access_key.as_deref().unwrap_or_default();
                    let sk = session.secret_key.as_deref().unwrap_or_default();
                    XdriveConfig::new_s3(endpoint, region, bucket, ak, sk, true, "aegis-xdrive")
                }
            };

            return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                .await;
        }
        XdriveWizardStep::FolderId => {
            let clean_folder = if let Some(pos) = raw_input.find("/folders/") {
                let after = &raw_input[pos + 9..];
                let end = after
                    .find(|c: char| {
                        c == '?' || c == '/' || c == '"' || c == '\'' || c.is_whitespace()
                    })
                    .unwrap_or(after.len());
                after[..end].trim().to_string()
            } else {
                raw_input.trim().to_string()
            };

            let folder_val = if session.region == "onedrive"
                && (clean_folder.is_empty() || clean_folder == "-" || clean_folder == "default")
            {
                "/xray_tunnel".to_string()
            } else {
                clean_folder
            };

            session.folder_id = Some(folder_val.clone());

            if session.region == "onedrive" {
                // 如果此前已成功自动提取了 Client ID 与凭据
                if let (Some(cid), Some(ref_tok)) = (
                    session.client_id.as_deref(),
                    session.refresh_token.as_deref(),
                ) {
                    let sec = session
                        .client_secret
                        .as_deref()
                        .unwrap_or(ONEDRIVE_DEFAULT_CLIENT_SECRET);
                    let sec_val = if sec.trim().is_empty()
                        || sec == "-"
                        || sec.eq_ignore_ascii_case("none")
                    {
                        ONEDRIVE_DEFAULT_CLIENT_SECRET
                    } else {
                        sec
                    };
                    let cid_val = if cid.trim().is_empty()
                        || cid == "-"
                        || cid.eq_ignore_ascii_case("none")
                    {
                        ONEDRIVE_DEFAULT_CLIENT_ID
                    } else {
                        cid
                    };
                    let cfg = if cid_val == ONEDRIVE_DEFAULT_CLIENT_ID
                        && sec_val == ONEDRIVE_DEFAULT_CLIENT_SECRET
                    {
                        XdriveConfig::new_onedrive(ref_tok, &folder_val)
                    } else {
                        XdriveConfig::new_onedrive_custom(cid_val, sec_val, ref_tok, &folder_val)
                    };
                    return complete_xdrive_node(
                        adapter,
                        target,
                        state,
                        &session,
                        &cfg,
                        cancel_markup,
                    )
                    .await;
                }

                session.step = XdriveWizardStep::ClientId;
                state.update_xdrive_wizard(&target.0, session).await;
                let prompt = "🟦 <b>Microsoft OneDrive 快速授权</b>\n✅ 中转文件夹已记录！\n\n👉 <b>推荐 30 秒极简授权：</b>\n在有浏览器的电脑终端运行：\n<code>rclone authorize \"onedrive\"</code>\n在弹出的浏览器完成微软账号登录，将生成的 <b>完整授权 JSON 代码</b> 直接贴回此处即可自动创建！\n\n（💡 <i>亦可输入自定义 Client ID 进入逐项配置模式</i>）";
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: prompt.to_string(),
                            markup: cancel_markup,
                        },
                    )
                    .await?;
                return Ok(MessageAction::Handled);
            }

            // 如果此前已成功自动提取了 Client ID 与凭据
            if let (Some(cid), Some(sec), Some(ref_tok), Some(folder)) = (
                session.client_id.as_deref(),
                session.client_secret.as_deref(),
                session.refresh_token.as_deref(),
                session.folder_id.as_deref(),
            ) {
                let cfg = XdriveConfig::new_google_drive(cid, sec, ref_tok, folder);
                return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                    .await;
            }

            session.step = XdriveWizardStep::ClientId;
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("xray.xdrive_guide_gdrive_step2").to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::ClientId => {
            session.client_id = Some(raw_input.to_string());
            session.step = XdriveWizardStep::ClientSecret;
            let prompt = if session.region == "onedrive" {
                t!("xray.xdrive_guide_onedrive_step3")
            } else {
                t!("xray.xdrive_guide_gdrive_step3")
            };
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: prompt.to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::ClientSecret => {
            let secret_val = if session.region == "onedrive"
                && (raw_input == "-"
                    || raw_input.eq_ignore_ascii_case("none")
                    || raw_input.is_empty())
            {
                "".to_string()
            } else {
                raw_input.to_string()
            };
            session.client_secret = Some(secret_val);
            session.step = XdriveWizardStep::RefreshToken;
            let prompt = if session.region == "onedrive" {
                t!("xray.xdrive_guide_onedrive_step4")
            } else {
                t!("xray.xdrive_guide_gdrive_step4")
            };
            state.update_xdrive_wizard(&target.0, session).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: prompt.to_string(),
                        markup: cancel_markup,
                    },
                )
                .await?;
        }
        XdriveWizardStep::RefreshToken => {
            session.refresh_token = Some(raw_input.to_string());

            let cid = session.client_id.as_deref().unwrap_or_default();
            let sec = session.client_secret.as_deref().unwrap_or_default();
            let ref_tok = session.refresh_token.as_deref().unwrap_or_default();
            let folder = session.folder_id.as_deref().unwrap_or_default();

            let cfg = if session.region == "onedrive" {
                let sec_val =
                    if sec.trim().is_empty() || sec == "-" || sec.eq_ignore_ascii_case("none") {
                        ONEDRIVE_DEFAULT_CLIENT_SECRET
                    } else {
                        sec
                    };
                let cid_val =
                    if cid.trim().is_empty() || cid == "-" || cid.eq_ignore_ascii_case("none") {
                        ONEDRIVE_DEFAULT_CLIENT_ID
                    } else {
                        cid
                    };
                let folder_val = if folder.is_empty() || folder == "-" || folder == "default" {
                    "/xray_tunnel"
                } else {
                    folder
                };
                if cid_val == ONEDRIVE_DEFAULT_CLIENT_ID
                    && sec_val == ONEDRIVE_DEFAULT_CLIENT_SECRET
                {
                    XdriveConfig::new_onedrive(ref_tok, folder_val)
                } else {
                    XdriveConfig::new_onedrive_custom(cid_val, sec_val, ref_tok, folder_val)
                }
            } else {
                XdriveConfig::new_google_drive(cid, sec, ref_tok, folder)
            };
            return complete_xdrive_node(adapter, target, state, &session, &cfg, cancel_markup)
                .await;
        }
    }

    Ok(MessageAction::Handled)
}

/// 自定义放行输入分支（添加）。
///
/// 单独成函数而不是塞进 ACME 的 match：该分支从校验到落盘都与 ACME 无关，
/// 混在一起后，任何未来的修改都得同时记住「这里还要判来源」。
async fn handle_custom_allowlist_input(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: Option<&str>,
    state: &dyn MessageState,
    target_str: &str,
) -> anyhow::Result<MessageAction> {
    let Some(input) = take_custom_input(adapter, target, text, state, target_str).await? else {
        return Ok(MessageAction::Handled);
    };

    let entry = match custom_allowlist_decision(&DomainFlowSource::CustomAllowlist, input) {
        Some(Ok(entry)) => entry,
        Some(Err(e)) => {
            send_plain(adapter, target, t!(custom_domain_error_key(&e)).to_string()).await?;
            // 非法输入不落盘、不清状态：用户重输一次即可，不必重进菜单。
            // 空输入已在上方先拦下，因此这里不会出现 Empty。
            return Ok(MessageAction::Handled);
        }
        // 实际不可达（上一行已固定传入 CustomAllowlist）；真出现时也只保持等待，
        // 因为「什么都不做」是唯一不会写坏配置的选择。
        None => return Ok(MessageAction::Handled),
    };

    match RoutingManager::add_custom_direct_entry(&entry).await {
        Ok(CustomAddOutcome::Added(domain)) => {
            send_plain(
                adapter,
                target,
                t!("xray.routing_custom_added", "domain" => domain).to_string(),
            )
            .await?;
        }
        Ok(CustomAddOutcome::AlreadyExists(domain)) => {
            send_plain(
                adapter,
                target,
                t!("xray.routing_custom_exists", "domain" => domain).to_string(),
            )
            .await?;
        }
        Ok(CustomAddOutcome::LimitReached) => {
            send_plain(adapter, target, t!("xray.routing_custom_full").to_string()).await?;
        }
        Err(e) => {
            // 写盘或 reload 失败：不向上抛错（上层会把异常当未处理消费掉），
            // 但必须清状态，否则用户会被永久困在输入态且以为还在等提示
            log::error!("add_custom_direct_entry failed: {e}");
            send_plain(
                adapter,
                target,
                t!("xray.routing_reload_failed").to_string(),
            )
            .await?;
        }
    }

    state.take_domain_input(target_str).await;
    Ok(MessageAction::Handled)
}

/// 从归一化条目里取出纯主机名：`domain:X` / `full:X` -> `X`。
///
/// 自检的匹配与展示都按主机名进行，而列表里存的是带前缀的条目；
/// 前缀在这里剥掉，而不是让 `match_custom_direct` 去理解条目前缀——
/// 判定函数一旦同时承担「前缀语义 + 域名语义」，日后加前缀就会静默漏匹配。
pub(crate) fn custom_entry_host(entry: &str) -> &str {
    entry
        .strip_prefix("domain:")
        .or_else(|| entry.strip_prefix("full:"))
        .unwrap_or(entry)
}

/// 自检结论（纯数据）：把「命中第几条」与「没命中」编码成值，
/// 便于让分支选择脱离文件系统被单独验证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CustomCheckOutcome {
    /// `idx` 是列表下标（展示时 +1），`entry` 是命中条目原文。
    Hit {
        idx: usize,
        entry: String,
    },
    Miss,
}

/// 只读判定：host 是否命中自定义放行列表。
/// 纯函数，不读盘、不写盘——自检的「只读」性质在这一层就成立。
pub(crate) fn custom_check_outcome(domains: &[String], host: &str) -> CustomCheckOutcome {
    match match_custom_direct(domains, host) {
        Some(idx) => CustomCheckOutcome::Hit {
            idx,
            entry: domains[idx].clone(),
        },
        None => CustomCheckOutcome::Miss,
    }
}

/// 渲染自检回报文案。
///
/// 被任一内建 direct 规则（connectivity_check「Google 服务直连」/ essential_direct
/// 「外网必需服务直连」…）命中的域名本来就会被放行：自检若只说「不在自定义列表里」，
/// 用户会误以为规则没生效而反复添加，因此额外追加一行说明它由哪条规则覆盖。
/// 规则名按 `xray.routing_rule_<id>` 约定现拼（三语文件不在本次改动的允许清单内）：
/// 规则名与菜单里的展示名同源，改规则名时提示不会过期。
fn custom_check_reply(outcome: &CustomCheckOutcome, host: &str) -> String {
    let mut text = match outcome {
        CustomCheckOutcome::Hit { idx, entry } => t!(
            "xray.routing_custom_check_hit",
            "idx" => (idx + 1).to_string(),
            "entry" => entry.clone()
        )
        .to_string(),
        CustomCheckOutcome::Miss => t!("xray.routing_custom_check_miss").to_string(),
    };

    // 报「命中的那条规则」而不是写死 connectivity_check：direct 规则不止一条，
    // 写死会把 essential_direct 覆盖的域名（recaptcha 等）张冠李戴。
    if let Some(rule_id) = matches_builtin_direct(host) {
        // 先绑定 key：`t!` 借用 key，直接把临时 `format!` 传进去会先被释放。
        let rule_name_key = format!("xray.routing_rule_{rule_id}");
        text.push_str("\nℹ️ ");
        text.push_str(t!(&rule_name_key).as_ref());
    }

    text
}

/// 自检输入分支。【只读】：不写盘、不 reload、不调 add_custom_direct_entry。
///
/// 全流程只有一次「读」（list_custom_direct_domains）+ 纯判定；
/// 自检的语义是「看一下现在的规则会怎么判」，任何写操作都会把查询变成修改。
async fn handle_custom_allowlist_check_input(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: Option<&str>,
    state: &dyn MessageState,
    target_str: &str,
) -> anyhow::Result<MessageAction> {
    let Some(input) = take_custom_input(adapter, target, text, state, target_str).await? else {
        return Ok(MessageAction::Handled);
    };

    let entry = match custom_allowlist_decision(&DomainFlowSource::CustomAllowlistCheck, input) {
        Some(Ok(entry)) => entry,
        Some(Err(e)) => {
            // 非法输入的提示与「添加」完全一致：同一个引导文案进来，不该有两种口径。
            send_plain(adapter, target, t!(custom_domain_error_key(&e)).to_string()).await?;
            return Ok(MessageAction::Handled);
        }
        None => return Ok(MessageAction::Handled),
    };

    // 自检的口径是主机名：归一化条目里剥掉 domain:/full: 前缀后再判定
    let host = custom_entry_host(&entry);

    match RoutingManager::list_custom_direct_domains().await {
        Ok(domains) => {
            let outcome = custom_check_outcome(&domains, host);
            send_plain(adapter, target, custom_check_reply(&outcome, host)).await?;
        }
        Err(e) => {
            // 读不到列表时【不能】报「未命中」：那会让用户以为规则失效而重复添加。
            // 复用「配置文件不存在」这条既有文案：它描述的正是此处唯一的失败原因。
            log::error!("list_custom_direct_domains failed: {e}");
            send_plain(adapter, target, t!("xray.user_cfg_not_found").to_string()).await?;
        }
    }

    state.take_domain_input(target_str).await;
    Ok(MessageAction::Handled)
}

async fn send_plain(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: String,
) -> anyhow::Result<()> {
    adapter
        .send_message(target, MessageContent { text, markup: None })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{
        BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
    };
    use crate::core::i18n;
    use crate::core::i18n::Lang;
    use crate::core::xray::routing::CustomDomainError;
    use crate::shared::types::TimeoutStatus;
    use anyhow::Result;
    use async_trait::async_trait;
    use serial_test::serial;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    struct FakeState {
        source: DomainFlowSource,
        chat_id: String,
        inner: Arc<Mutex<FakeStateInner>>,
        xdrive: Arc<Mutex<std::collections::HashMap<String, XdriveWizardSession>>>,
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
                xdrive: Arc::new(Mutex::new(std::collections::HashMap::new())),
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
                xdrive: Arc::new(Mutex::new(std::collections::HashMap::new())),
            }
        }

        /// 自检来源：与添加共用 AwaitDomain 输入态，但只能走只读判定分支。
        fn custom_check(step: DomainInputStep) -> Self {
            Self {
                source: DomainFlowSource::CustomAllowlistCheck,
                chat_id: "test_chat".to_string(),
                inner: Arc::new(Mutex::new(FakeStateInner { step, domain: None })),
                xdrive: Arc::new(Mutex::new(std::collections::HashMap::new())),
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

        async fn get_xdrive_wizard(&self, target: &str) -> Option<XdriveWizardSession> {
            self.xdrive.lock().unwrap().get(target).cloned()
        }

        async fn update_xdrive_wizard(&self, target: &str, session: XdriveWizardSession) {
            self.xdrive
                .lock()
                .unwrap()
                .insert(target.to_string(), session);
        }

        async fn clear_xdrive_wizard(&self, target: &str) {
            self.xdrive.lock().unwrap().remove(target);
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
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities::TELEGRAM
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

    /// Mutates the process-global locale; see the note on
    /// `empty_domain_keeps_await_domain_state`.
    #[serial]
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

    /// Mutates the process-global locale; see the note on
    /// `empty_domain_keeps_await_domain_state`.
    #[serial]
    #[test]
    fn acme_install_failure_hides_arbitrary_detail() {
        i18n::set_lang(Lang::En);

        let rendered = localized_acme_install_failure();

        assert!(rendered.contains("ACME-UNKNOWN"));
        assert!(!rendered.contains("domain.acme_install_fail"));
    }

    #[serial]
    #[test]
    fn new_issuance_uses_existing_issuance_message() {
        assert_eq!(
            certificate_progress_message("example.com", AcmeCertificateOperation::Issue),
            t!("domain.issuing_cert", "0" => "example.com").to_string()
        );
    }

    #[serial]
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

    /// Asserts an exact localized string, so it races any concurrent
    /// `set_lang`/`set_locale`. `rust-i18n`'s locale is process-global and
    /// nextest's per-process isolation hides that, while plain `cargo test`
    /// does not. Every locale writer in this crate is `#[serial]` so this
    /// holds.
    #[serial]
    #[tokio::test]
    async fn empty_domain_keeps_await_domain_state() {
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::domain(DomainInputStep::AwaitDomain);

        // locale 必须在被测调用**之前**设置：`handle_message` 内部按当时的
        // 进程全局 locale 渲染文案，若放到调用之后，断言看到的其实是上一个
        // 测试残留的语言。曾经这条 set_lang 写在 handle_message 之后，
        // 在 plain `cargo test` 下会随测试执行顺序随机失败。
        i18n::set_lang(Lang::En);
        let action = handle_message(&adapter, &target, Some("  "), false, &state)
            .await
            .unwrap();

        assert!(matches!(action, MessageAction::Handled));
        assert!(matches!(state.snapshot(), DomainInputStep::AwaitDomain));
        assert_eq!(
            adapter.last_text(),
            "Domain cannot be empty, please re-enter."
        );
    }

    /// Asserts an exact localized string; see the note on
    /// `empty_domain_keeps_await_domain_state`.
    #[serial]
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

    // ── 自定义放行输入分流（T6a）─────────────────────────────────────────
    // 「自定义放行」复用同一个 AwaitDomain 输入状态，但绝不能落进 ACME 路径：
    // 以下断言就是「不干扰 ACME」的锁死线。

    /// 只有 CustomAllowlist 来源才返回判定结果；ACME 的两个来源必须返回 None，
    /// 否则 ACME 域名输入会被当成「放行域名」处理（或反之，放行输入触发证书签发）。
    #[test]
    fn acme_sources_never_enter_custom_allowlist() {
        for source in [DomainFlowSource::Standalone, DomainFlowSource::OneClick] {
            assert_eq!(
                custom_allowlist_decision(&source, "decodo.cn"),
                None,
                "{source:?} 必须完全不进自定义放行分支"
            );
            assert_eq!(custom_allowlist_decision(&source, "https://x/y"), None);
        }
    }

    #[test]
    fn custom_allowlist_source_normalizes_plain_domain() {
        assert_eq!(
            custom_allowlist_decision(&DomainFlowSource::CustomAllowlist, " decodo.cn "),
            Some(Ok("domain:decodo.cn".to_string()))
        );
    }

    #[test]
    fn custom_allowlist_source_surfaces_normalizer_errors() {
        assert_eq!(
            custom_allowlist_decision(&DomainFlowSource::CustomAllowlist, "https://x/y"),
            Some(Err(CustomDomainError::HasSchemeOrPath))
        );
    }

    #[test]
    fn custom_domain_error_keys_cover_every_variant() {
        use CustomDomainError::*;

        assert_eq!(
            custom_domain_error_key(&HasSchemeOrPath),
            "xray.routing_custom_invalid_scheme"
        );
        assert_eq!(
            custom_domain_error_key(&SingleLabel),
            "xray.routing_custom_invalid_single_label"
        );
        assert_eq!(
            custom_domain_error_key(&IpNotSupported),
            "xray.routing_custom_invalid_ip"
        );
        // 其余原因共用通用文案：用户能做的动作相同（重输一个合法域名），
        // 区分文案只会增加三语维护成本
        for e in [
            Empty,
            HasPort,
            InvalidLabel,
            TooLong,
            UnsupportedPrefix,
            NonAscii,
        ] {
            assert_eq!(
                custom_domain_error_key(&e),
                "xray.routing_custom_invalid_generic"
            );
        }
    }

    /// 映射出的 key 必须真的存在：rust-i18n 找不到 key 时会把 key 原样返回给用户，
    /// 拼错一个字母就会把 `xray.routing_custom_...` 直接显示出来。
    #[test]
    fn custom_domain_error_keys_exist_in_all_locales() {
        use CustomDomainError::*;

        let keys = [
            custom_domain_error_key(&HasSchemeOrPath),
            custom_domain_error_key(&SingleLabel),
            custom_domain_error_key(&IpNotSupported),
            custom_domain_error_key(&Empty),
        ];
        for yaml in [
            include_str!("../../resources/i18n/zh.yml"),
            include_str!("../../resources/i18n/en.yml"),
            include_str!("../../resources/i18n/ja.yml"),
        ] {
            for key in keys {
                // yml 里只写叶子 key（缩进在 `xray:` 段内），断言时去掉段名前缀
                let leaf = key.strip_prefix("xray.").expect("key 应带 xray. 段名");
                assert!(
                    yaml.contains(&format!("\n  {leaf}: ")),
                    "缺少 i18n key: {key}"
                );
            }
        }
    }

    // ── 生效自检（T7b）：只读判定 ────────────────────────────────────────
    // 自检与添加共用同一个输入状态机，区别只在「判定 vs 写盘」。以下断言分两层：
    // 纯逻辑（host 提取 / 命中分支选择）与分流锁死线。

    /// 锁死线：CustomAllowlistCheck 必须和 CustomAllowlist 一样在【进入 ACME 分支之前】
    /// 被拦下；而 ACME 的两个来源仍必须返回 None，否则证书流程会被自检输入污染。
    #[test]
    fn custom_allowlist_check_never_enters_acme_path() {
        assert_eq!(
            custom_allowlist_decision(&DomainFlowSource::CustomAllowlistCheck, " decodo.cn "),
            Some(Ok("domain:decodo.cn".to_string()))
        );
        assert_eq!(
            custom_allowlist_decision(&DomainFlowSource::CustomAllowlistCheck, "https://x/y"),
            Some(Err(CustomDomainError::HasSchemeOrPath))
        );
        for source in [DomainFlowSource::Standalone, DomainFlowSource::OneClick] {
            assert_eq!(
                custom_allowlist_decision(&source, "decodo.cn"),
                None,
                "{source:?} 不得进入自定义放行/自检分支"
            );
        }
    }

    /// 自检判定用的是纯主机名：列表里存的是 `domain:`/`full:` 条目，
    /// 前缀在这里剥掉而不是让 match_custom_direct 去理解前缀（否则判定函数要同时
    /// 处理两种语义，任何一处漏改都会静默漏匹配）。
    #[test]
    fn custom_entry_host_strips_known_prefixes() {
        assert_eq!(custom_entry_host("domain:decodo.cn"), "decodo.cn");
        assert_eq!(custom_entry_host("full:decodo.cn"), "decodo.cn");
        assert_eq!(custom_entry_host("decodo.cn"), "decodo.cn");
    }

    /// 命中分支选择 + host 提取合起来就是自检的全部判定逻辑。
    /// 之所以把它做成纯函数并单测：自检分支【不写盘】这一点无法在集成测试里直接断言，
    /// 至少要保证判定本身可被脱离文件系统验证。
    #[test]
    fn custom_check_outcome_selects_hit_or_miss() {
        let list = vec![
            "domain:decodo.cn".to_string(),
            "full:exact.example".to_string(),
        ];
        assert_eq!(
            custom_check_outcome(&list, custom_entry_host("domain:decodo.cn")),
            CustomCheckOutcome::Hit {
                idx: 0,
                entry: "domain:decodo.cn".to_string()
            }
        );
        // `domain:` 条目命中其子域
        assert_eq!(
            custom_check_outcome(&list, "a.b.decodo.cn"),
            CustomCheckOutcome::Hit {
                idx: 0,
                entry: "domain:decodo.cn".to_string()
            }
        );
        // `full:` 条目仅精确匹配，子域必须判未命中
        assert_eq!(
            custom_check_outcome(&list, custom_entry_host("full:exact.example")),
            CustomCheckOutcome::Hit {
                idx: 1,
                entry: "full:exact.example".to_string()
            }
        );
        assert_eq!(
            custom_check_outcome(&list, "sub.exact.example"),
            CustomCheckOutcome::Miss
        );
        assert_eq!(
            custom_check_outcome(&[], "decodo.cn"),
            CustomCheckOutcome::Miss
        );
    }

    /// 命中回报要带「人性化序号」（下标 +1）与命中条目本身：这两个值直接展示给用户。
    /// 断言精确字符串会与并发 set_lang 竞争，故 #[serial]（同 empty_domain_keeps_await_domain_state 的说明）；
    /// 结束时把语言恢复原样，避免污染后来者对「默认语言」的隐含假设。
    #[serial]
    #[test]
    fn custom_check_reply_renders_hit_index_and_entry() {
        let previous = i18n::current_lang();
        i18n::set_lang(Lang::Zh);
        let hit = CustomCheckOutcome::Hit {
            idx: 2,
            entry: "domain:decodo.cn".to_string(),
        };
        assert_eq!(
            custom_check_reply(&hit, "decodo.cn"),
            t!(
                "xray.routing_custom_check_hit",
                "idx" => "3",
                "entry" => "domain:decodo.cn"
            )
            .to_string()
        );
        i18n::set_lang(previous);
    }

    /// 被 connectivity_check（Google 服务直连）命中的域名本来就会被放行，
    /// 自检必须额外说明，否则用户会把「不在自定义列表里」理解成「没生效」而反复添加。
    #[serial]
    #[test]
    fn custom_check_reply_appends_connectivity_rule_note() {
        let previous = i18n::current_lang();
        i18n::set_lang(Lang::Zh);
        let miss = CustomCheckOutcome::Miss;

        // 用规则里真实存在的探测域名（见 routing.rs 的 connectivity_check targets）：
        // 断言依赖的是规则内容本身，而不是另抄一份域名清单。
        let google = custom_check_reply(&miss, "connectivitycheck.gstatic.com");
        assert!(google.contains(&t!("xray.routing_custom_check_miss").to_string()));
        assert!(
            google.contains(&t!("xray.routing_rule_connectivity_check").to_string()),
            "应说明它由 Google 服务直连规则覆盖: {google}"
        );

        // 非 connectivity 域名不得出现该提示，否则提示会退化成噪音
        let other = custom_check_reply(&miss, "example.com");
        assert!(!other.contains(&t!("xray.routing_rule_connectivity_check").to_string()));
        i18n::set_lang(previous);
    }

    /// 自检提示必须用「命中的那条内建 direct 规则」的名字：direct 规则不止
    /// connectivity_check 一条，硬编码 key 会把 essential_direct 覆盖的域名
    /// （如 recaptcha）张冠李戴地报成「Google 服务直连」。
    #[serial]
    #[test]
    fn custom_check_reply_uses_hit_rule_name() {
        let previous = i18n::current_lang();
        i18n::set_lang(Lang::Zh);
        let miss = CustomCheckOutcome::Miss;

        // domain:recaptcha.net 只被 essential_direct 覆盖
        let essential = custom_check_reply(&miss, "www.recaptcha.net");
        assert!(
            essential.contains(&t!("xray.routing_rule_essential_direct").to_string()),
            "应回显命中的 essential_direct 规则名: {essential}"
        );
        assert!(
            !essential.contains(&t!("xray.routing_rule_connectivity_check").to_string()),
            "不得报成 Google 服务直连: {essential}"
        );
        i18n::set_lang(previous);
    }

    /// 自检来源走只读分支：不产出 DomainReady（不会去签证书/建站）、
    /// 不推进 ACME 输入步骤、不改动 00_base.json。
    #[serial]
    #[tokio::test]
    async fn check_source_is_read_only_and_leaves_acme_state_untouched() {
        let adapter = RecordingAdapter::new();
        let target = TargetId("test_chat".to_string());
        let state = FakeState::custom_check(DomainInputStep::AwaitDomain);

        let base = format!("{}/00_base.json", crate::core::paths::xray::CONF_DIR);
        let before = std::fs::read(&base).ok();
        let action = handle_message(&adapter, &target, Some("decodo.cn"), false, &state)
            .await
            .unwrap();
        let after = std::fs::read(&base).ok();

        assert!(
            matches!(action, MessageAction::Handled),
            "自检绝不能产出 DomainReady"
        );
        // ACME 分支会把输入推进到 AwaitProvider/Processing；自检必须原地不动
        assert!(
            matches!(state.snapshot(), DomainInputStep::AwaitDomain),
            "自检不得推进 ACME 输入状态"
        );
        assert_eq!(before, after, "自检不得改动 {base}");
    }

    /// 自检分支复用了既有 i18n key（三语文件不在本次允许改动的清单内，故不新增 key）：
    /// 规则名按 `xray.routing_rule_<id>` 现拼，故所有会命中自检的 direct 规则 id
    /// 都必须三语齐备，否则用户会直接看到 `xray.routing_rule_essential_direct` 这种原始 key。
    #[test]
    fn custom_check_reused_keys_exist_in_all_locales() {
        for yaml in [
            include_str!("../../resources/i18n/zh.yml"),
            include_str!("../../resources/i18n/en.yml"),
            include_str!("../../resources/i18n/ja.yml"),
        ] {
            for leaf in [
                "routing_rule_connectivity_check",
                "routing_rule_essential_direct",
                "user_cfg_not_found",
            ] {
                assert!(
                    yaml.contains(&format!("\n  {leaf}: ")),
                    "缺少 i18n key: xray.{leaf}"
                );
            }
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

    #[tokio::test]
    async fn test_xdrive_wizard_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        // Initialize RU wizard session
        let session = XdriveWizardSession {
            region: "ru".to_string(),
            step: XdriveWizardStep::Bucket,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Bucket name
        let action = handle_message(&adapter, &target, Some("my-bucket"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::AccessKey);
        assert_eq!(session.bucket.as_deref(), Some("my-bucket"));

        // Step 2: User sends Access Key
        let action = handle_message(&adapter, &target, Some("my-ak-12345"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::SecretKey);
        assert_eq!(session.access_key.as_deref(), Some("my-ak-12345"));

        // Step 3: Test cancel via /cancel
        let action = handle_message(&adapter, &target, Some("/cancel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_cloudflare_auto_extract() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "cn".to_string(),
            step: XdriveWizardStep::AccountId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let sample = r#"
R2 Account Token 已成功创建
令牌值
cfat_mocktoken1234567890abcdef1234567890
为 S3 客户端使用以下凭据：
访问密钥 ID
b65ef1b0f6dcc85d9c2e42a92610fe14
机密访问密钥
4b07c8ef6340317e0b51fae296837bfdbddbfcf92e071727c955da3597dff758
为 S3 客户端使用管辖权地特定的终结点：
https://a5be0a9d14aa44e51692d81782d83fe6.r2.cloudflarestorage.com
"#;

        let action = handle_message(&adapter, &target, Some(sample), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(
            session.account_id.as_deref(),
            Some("a5be0a9d14aa44e51692d81782d83fe6")
        );
        assert_eq!(
            session.access_key.as_deref(),
            Some("b65ef1b0f6dcc85d9c2e42a92610fe14")
        );
        assert_eq!(
            session.secret_key.as_deref(),
            Some("4b07c8ef6340317e0b51fae296837bfdbddbfcf92e071727c955da3597dff758")
        );
    }

    #[tokio::test]
    async fn test_xdrive_wizard_cloudflare_custom_domain() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        // Test custom domain string (e.g. r2.mycompany.com)
        let session = XdriveWizardSession {
            region: "cn".to_string(),
            step: XdriveWizardStep::AccountId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let action = handle_message(&adapter, &target, Some("r2.mycompany.com"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(session.endpoint.as_deref(), Some("r2.mycompany.com"));
        assert_eq!(session.account_id, None);

        // Test custom endpoint URL with https
        let session = XdriveWizardSession {
            region: "r2".to_string(),
            step: XdriveWizardStep::AccountId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let action = handle_message(
            &adapter,
            &target,
            Some("https://custom-r2.example.com/"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(
            session.endpoint.as_deref(),
            Some("https://custom-r2.example.com")
        );
        assert_eq!(session.account_id, None);

        // Test standard 32-hex Account ID
        let session = XdriveWizardSession {
            region: "cn".to_string(),
            step: XdriveWizardStep::AccountId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let action = handle_message(
            &adapter,
            &target,
            Some("a5be0a9d14aa44e51692d81782d83fe6"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(
            session.account_id.as_deref(),
            Some("a5be0a9d14aa44e51692d81782d83fe6")
        );
        assert_eq!(session.endpoint, None);
    }

    #[tokio::test]
    async fn test_xdrive_wizard_google_drive_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        // Initialize Google Drive wizard session
        let session = XdriveWizardSession {
            region: "gdrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Folder URL
        let action = handle_message(
            &adapter,
            &target,
            Some("https://drive.google.com/drive/folders/1ABC_folder_XYZ?usp=sharing"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::ClientId);
        assert_eq!(session.folder_id.as_deref(), Some("1ABC_folder_XYZ"));

        // Step 2: User sends Client ID
        let action = handle_message(
            &adapter,
            &target,
            Some("123456.apps.googleusercontent.com"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::ClientSecret);
        assert_eq!(
            session.client_id.as_deref(),
            Some("123456.apps.googleusercontent.com")
        );

        // Step 3: User sends Client Secret
        let action = handle_message(
            &adapter,
            &target,
            Some("GOCSPX-secret_abc_123"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::RefreshToken);
        assert_eq!(
            session.client_secret.as_deref(),
            Some("GOCSPX-secret_abc_123")
        );

        // Step 4: User cancels via /cancel
        let action = handle_message(&adapter, &target, Some("/cancel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_google_drive_auto_extract() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "gdrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let sample = r#"
https://developers.google.com/oauthplayground
Client ID: 123456789.apps.googleusercontent.com
Client Secret: {{SECRET_h6u8z0r0}}
Refresh Token: 1//0gSampleRefreshToken
"#;

        let action = handle_message(&adapter, &target, Some(sample), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::FolderId);
        assert_eq!(
            session.client_id.as_deref(),
            Some("123456789.apps.googleusercontent.com")
        );
        assert_eq!(
            session.client_secret.as_deref(),
            Some("{{SECRET_h6u8z0r0}}")
        );
        assert_eq!(
            session.refresh_token.as_deref(),
            Some("1//0gSampleRefreshToken")
        );

        let last_msg = adapter.last_text();
        assert!(
            last_msg.contains("Google Drive")
                && last_msg.contains("123456789.apps.googleusercontent.com")
        );
    }

    #[tokio::test]
    async fn test_xdrive_wizard_google_drive_auto_extract_json() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "gdrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let json_sample = r#"{
            "client_id": "json-test.apps.googleusercontent.com",
            "client_secret": "{{SECRET_e2g949l1}}",
            "refresh_token": "1//json-refresh-sample"
        }"#;

        let action = handle_message(&adapter, &target, Some(json_sample), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::FolderId);
        assert_eq!(
            session.client_id.as_deref(),
            Some("json-test.apps.googleusercontent.com")
        );
        assert_eq!(
            session.client_secret.as_deref(),
            Some("{{SECRET_e2g949l1}}")
        );
        assert_eq!(
            session.refresh_token.as_deref(),
            Some("1//json-refresh-sample")
        );
    }

    #[tokio::test]
    async fn test_xdrive_wizard_aws_s3_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "aws".to_string(),
            step: XdriveWizardStep::Region,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Region
        let action = handle_message(&adapter, &target, Some("ap-northeast-1"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(session.s3_region.as_deref(), Some("ap-northeast-1"));
        let msg = adapter.last_text();
        assert!(msg.contains("AWS S3") || msg.contains("存储桶") || msg.contains("Bucket"));

        // Step 2: User sends Bucket
        let action = handle_message(&adapter, &target, Some("my-aws-bucket"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::AccessKey);
        assert_eq!(session.bucket.as_deref(), Some("my-aws-bucket"));

        // Step 3: User sends Access Key
        let action = handle_message(
            &adapter,
            &target,
            Some("AKIAIOSFODNN7EXAMPLE"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::SecretKey);
        assert_eq!(session.access_key.as_deref(), Some("AKIAIOSFODNN7EXAMPLE"));

        // Step 4: User completes by sending Secret Key
        let action = handle_message(
            &adapter,
            &target,
            Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        // Wizard session is cleared upon completion
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_aws_s3_auto_extract_without_bucket() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "aws".to_string(),
            step: XdriveWizardStep::Region,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let ini_credentials = r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
region = us-east-1
"#;

        let action = handle_message(&adapter, &target, Some(ini_credentials), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(session.s3_region.as_deref(), Some("us-east-1"));
        assert_eq!(session.access_key.as_deref(), Some("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(
            session.secret_key.as_deref(),
            Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")
        );

        let last_msg = adapter.last_text();
        assert!(
            last_msg.contains("AWS S3")
                && last_msg.contains("us-east-1")
                && last_msg.contains("AKIAIOSFODNN7EXAMPLE")
        );

        // Advance: User inputs bucket name to finalize
        let action = handle_message(&adapter, &target, Some("my-final-bucket"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_aws_s3_auto_extract_with_bucket() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "aws".to_string(),
            step: XdriveWizardStep::Region,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let json_credentials = r#"{
            "access_key_id": "AKIAIOSFODNN7EXAMPLE",
            "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "region": "ap-southeast-1",
            "bucket": "auto-extracted-bucket"
        }"#;

        let action = handle_message(&adapter, &target, Some(json_credentials), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        // Since bucket was included in the JSON, it completes immediately and clears wizard
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_oracle_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "oracle".to_string(),
            step: XdriveWizardStep::AccountId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Namespace
        let action = handle_message(&adapter, &target, Some("ax1234567890"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Region);
        assert_eq!(session.account_id.as_deref(), Some("ax1234567890"));

        // Step 2: User sends Region
        let action = handle_message(&adapter, &target, Some("ap-chuncheon-1"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(session.s3_region.as_deref(), Some("ap-chuncheon-1"));
    }

    #[tokio::test]
    async fn test_xdrive_wizard_b2_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "b2".to_string(),
            step: XdriveWizardStep::Region,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Region
        let action = handle_message(&adapter, &target, Some("us-west-004"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::Bucket);
        assert_eq!(session.s3_region.as_deref(), Some("us-west-004"));
    }

    #[tokio::test]
    async fn test_xdrive_wizard_onedrive_conversational_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "onedrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Folder Path
        let action = handle_message(&adapter, &target, Some("/xray_tunnel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::ClientId);
        assert_eq!(session.folder_id.as_deref(), Some("/xray_tunnel"));

        // Step 2: User sends Client ID
        let action = handle_message(
            &adapter,
            &target,
            Some("11111111-2222-3333-4444-555555555555"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::ClientSecret);
        assert_eq!(
            session.client_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );

        // Step 3: User sends Client Secret
        let action = handle_message(
            &adapter,
            &target,
            Some("my_onedrive_secret_abc"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::RefreshToken);
        assert_eq!(
            session.client_secret.as_deref(),
            Some("my_onedrive_secret_abc")
        );

        // Step 4: User sends Refresh Token and completes node creation
        let action = handle_message(
            &adapter,
            &target,
            Some("0.A.sample_refresh_token_xyz"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        // Wizard session is cleared upon successful completion
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_onedrive_public_client_skip_secret() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "onedrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // Step 1: User sends Folder Path
        let action = handle_message(&adapter, &target, Some("/xray_tunnel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        // Step 2: User sends Client ID
        let action = handle_message(
            &adapter,
            &target,
            Some("public-client-id-uuid"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        // Step 3: User sends "-" to skip client secret (public client)
        let action = handle_message(&adapter, &target, Some("-"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::RefreshToken);
        assert_eq!(session.client_secret.as_deref(), Some(""));

        // Step 4: User sends Refresh Token and completes node creation
        let action = handle_message(
            &adapter,
            &target,
            Some("public_refresh_token_abc"),
            false,
            &state,
        )
        .await
        .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        // Wizard session is cleared upon successful completion
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[tokio::test]
    async fn test_xdrive_wizard_onedrive_auto_extract() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "onedrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        let sample = r#"{
            "client_id": "auto-od-client-id",
            "client_secret": "-",
            "refresh_token": "auto-od-refresh-token"
        }"#;

        let action = handle_message(&adapter, &target, Some(sample), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::FolderId);
        assert_eq!(session.client_id.as_deref(), Some("auto-od-client-id"));
        assert_eq!(
            session.client_secret.as_deref(),
            Some(ONEDRIVE_DEFAULT_CLIENT_SECRET)
        );
        assert_eq!(
            session.refresh_token.as_deref(),
            Some("auto-od-refresh-token")
        );

        let last_msg = adapter.last_text();
        assert!(last_msg.contains("OneDrive") && last_msg.contains("auto-od-client-id"));

        // Follow up with folder to complete
        let action = handle_message(&adapter, &target, Some("/my_tunnel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }

    #[test]
    fn test_extract_onedrive_tokens_rclone_json_defaults() {
        // 1. Raw JSON containing only refresh_token and access_token (typical rclone output)
        let rclone_json = r#"{
            "access_token": "EwBAA8l6BAAU...",
            "token_type": "Bearer",
            "refresh_token": "0.ARwA111_sample_refresh_token_xyz",
            "expiry": "2026-11-10T12:00:00Z"
        }"#;

        let (cid, sec, ref_tok, folder) = extract_onedrive_tokens(rclone_json)
            .expect("should successfully extract tokens from rclone authorize JSON");
        assert_eq!(cid, ONEDRIVE_DEFAULT_CLIENT_ID);
        assert_eq!(sec, ONEDRIVE_DEFAULT_CLIENT_SECRET);
        assert_eq!(ref_tok, "0.ARwA111_sample_refresh_token_xyz");
        assert_eq!(folder, None);

        // 2. rclone CLI output with surrounding banner lines
        let rclone_banner_cli = r#"
Paste the following into your remote machine --->
{"access_token":"eyJ...","token_type":"Bearer","refresh_token":"0.ARwA222_banner_refresh_token","expiry":"2026-11-10T15:00:00.0000000+08:00"}
<---End paste
"#;
        let (cid, sec, ref_tok, _) = extract_onedrive_tokens(rclone_banner_cli)
            .expect("should extract from rclone CLI output with banners");
        assert_eq!(cid, ONEDRIVE_DEFAULT_CLIENT_ID);
        assert_eq!(sec, ONEDRIVE_DEFAULT_CLIENT_SECRET);
        assert_eq!(ref_tok, "0.ARwA222_banner_refresh_token");

        // 3. Nested token JSON (e.g. from rclone config dump)
        let nested_json = r#"{
            "token": "{\"access_token\":\"at\",\"refresh_token\":\"0.ARwA333_nested\"}"
        }"#;
        let (cid, sec, ref_tok, _) =
            extract_onedrive_tokens(nested_json).expect("should extract from nested token string");
        assert_eq!(cid, ONEDRIVE_DEFAULT_CLIENT_ID);
        assert_eq!(sec, ONEDRIVE_DEFAULT_CLIENT_SECRET);
        assert_eq!(ref_tok, "0.ARwA333_nested");
    }

    #[test]
    fn test_extract_onedrive_tokens_custom_credentials() {
        // 1. JSON with custom client_id and client_secret
        let custom_json = r#"{
            "client_id": "custom-app-id-123",
            "client_secret": "custom-secret-456",
            "refresh_token": "custom-ref-tok-789",
            "folder": "/custom_tunnel"
        }"#;
        let (cid, sec, ref_tok, folder) = extract_onedrive_tokens(custom_json)
            .expect("should extract custom credentials from JSON");
        assert_eq!(cid, "custom-app-id-123");
        assert_eq!(sec, "custom-secret-456");
        assert_eq!(ref_tok, "custom-ref-tok-789");
        assert_eq!(folder.as_deref(), Some("/custom_tunnel"));

        // 2. Multi-line key-value text with custom credentials
        let text_input = r#"
client_id = my-text-client-id
client_secret = my-text-client-secret
refresh_token = my-text-refresh-token
folder = /text_tunnel
"#;
        let (cid, sec, ref_tok, folder) = extract_onedrive_tokens(text_input)
            .expect("should extract custom credentials from key-value text");
        assert_eq!(cid, "my-text-client-id");
        assert_eq!(sec, "my-text-client-secret");
        assert_eq!(ref_tok, "my-text-refresh-token");
        assert_eq!(folder.as_deref(), Some("/text_tunnel"));

        // 3. Multi-line text with custom client_id but omitted secret (fallback to default)
        let text_omit_sec = r#"
client_id = my-custom-id
client_secret = -
refresh_token = my-custom-token
"#;
        let (cid, sec, ref_tok, _) = extract_onedrive_tokens(text_omit_sec)
            .expect("should fallback secret to default when omitted");
        assert_eq!(cid, "my-custom-id");
        assert_eq!(sec, ONEDRIVE_DEFAULT_CLIENT_SECRET);
        assert_eq!(ref_tok, "my-custom-token");
    }

    #[tokio::test]
    async fn test_xdrive_wizard_onedrive_rclone_authorize_flow() {
        let adapter = RecordingAdapter::new();
        let state = FakeState::domain(DomainInputStep::AwaitDomain);
        let target = TargetId("test_chat".into());

        let session = XdriveWizardSession {
            region: "onedrive".to_string(),
            step: XdriveWizardStep::FolderId,
            account_id: None,
            endpoint: None,
            s3_region: None,
            bucket: None,
            access_key: None,
            secret_key: None,
            client_id: None,
            client_secret: None,
            refresh_token: None,
            folder_id: None,
            started_at: std::time::Instant::now(),
        };
        state.update_xdrive_wizard("test_chat", session).await;

        // User pastes rclone authorize JSON directly into wizard
        let rclone_output = r#"{
            "access_token": "sample_access_token_at",
            "token_type": "Bearer",
            "refresh_token": "0.ARwA_rclone_refresh_token_abc",
            "expiry": "2026-11-10T12:00:00Z"
        }"#;

        let action = handle_message(&adapter, &target, Some(rclone_output), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        let session = state.get_xdrive_wizard("test_chat").await.unwrap();
        assert_eq!(session.step, XdriveWizardStep::FolderId);
        assert_eq!(
            session.client_id.as_deref(),
            Some(ONEDRIVE_DEFAULT_CLIENT_ID)
        );
        assert_eq!(
            session.client_secret.as_deref(),
            Some(ONEDRIVE_DEFAULT_CLIENT_SECRET)
        );
        assert_eq!(
            session.refresh_token.as_deref(),
            Some("0.ARwA_rclone_refresh_token_abc")
        );

        // User responds with folder name
        let action = handle_message(&adapter, &target, Some("/my_od_tunnel"), false, &state)
            .await
            .unwrap();
        assert!(matches!(action, MessageAction::Handled));

        // Wizard session is cleared upon successful node creation
        assert!(state.get_xdrive_wizard("test_chat").await.is_none());
    }
}
