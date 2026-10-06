use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::common::{BotAdapter, InlineButton, Markup, MessageContent, TargetId};
use crate::core::security::acme::{
    AcmeCertificateOperation, AcmeCommandError, AcmeFailureKind, AcmeManager, XhttpDeployMode,
};
use crate::core::types::{DnsProvider, DomainFlowSource, DomainInputState, DomainInputStep};
use crate::core::xray::config::ConfigManager;
use crate::core::xray::routing::{
    CustomAddOutcome, RoutingManager, match_custom_direct, matches_builtin_direct,
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
/// 「外網必需服務直連」…）命中的域名本来就会被放行：自检若只说「不在自定义列表里」，
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

        /// 自检来源：与添加共用 AwaitDomain 输入态，但只能走只读判定分支。
        fn custom_check(step: DomainInputStep) -> Self {
            Self {
                source: DomainFlowSource::CustomAllowlistCheck,
                chat_id: "test_chat".to_string(),
                inner: Arc::new(Mutex::new(FakeStateInner { step, domain: None })),
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
}
