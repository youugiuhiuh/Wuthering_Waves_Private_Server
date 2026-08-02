use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sha2::Digest;

use crate::app::auth;
use crate::app::state::AppState;
use crate::common::{MessageContent, MessageId};
use crate::core::security::acme::XhttpDeployMode;
use crate::core::system::host_settings::SystemHostSettings;
use crate::core::types::DomainFlowSource;
use crate::shared::handlers::message::{self, MessageAction};
use crate::shared::types::{
    BotCommand, BotEvent, CallbackEvent, CommandEvent, HandlerAction, MessageEvent, TimeoutStatus,
};
use crate::shared::{commands, destruct, handlers, state_ops};

#[allow(dead_code)]
pub async fn dispatch_event(event: BotEvent, state: &AppState) -> Result<()> {
    // 1. Destruct flow interception (checks timeout, handles in-progress destruct)
    match &event {
        BotEvent::Message(msg) => {
            if destruct::intercept_message(msg, state).await? == destruct::FlowOutcome::Handled {
                return Ok(());
            }
        }
        BotEvent::Callback(cb) => {
            if destruct::intercept_callback(cb, state).await? == destruct::FlowOutcome::Handled {
                return Ok(());
            }
        }
        BotEvent::Command(_) => {}
    }

    // 2. Authorization check
    if !check_auth(&event, state).await {
        return Ok(());
    }

    // 3. Dispatch by event type
    match event {
        BotEvent::Command(cmd) => {
            commands::handle(cmd, state).await?;
        }
        BotEvent::Message(msg) => {
            handle_message(msg, state).await?;
        }
        BotEvent::Callback(mut cb) => {
            // State operations (lang, set_timeout, warp input). The lang branch
            // may return a redirect (e.g. re-show main menu).
            if let Some(next) = state_ops::intercept(&cb, state, Arc::new(SystemHostSettings)).await
            {
                cb = CallbackEvent { data: next, ..cb };
            }
            // Shared callback dispatch (from Phase A), following redirects for
            // multi-step menu flows.
            let mut current = cb;
            let mut iterations = 0usize;
            loop {
                iterations += 1;
                if iterations > 16 {
                    break;
                }
                match handlers::dispatch(&current, state).await? {
                    Some(HandlerAction::Redirect(data)) => {
                        current = CallbackEvent { data, ..current };
                    }
                    _ => break,
                }
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
async fn check_auth(event: &BotEvent, state: &AppState) -> bool {
    let Ok(user_id) = event.user_id() else {
        return false;
    };
    if !state.is_admin_user(user_id) {
        return false;
    }
    match event {
        BotEvent::Command(CommandEvent {
            command: BotCommand::Auth { .. },
            ..
        }) => true,
        BotEvent::Command(CommandEvent {
            command: BotCommand::Help | BotCommand::Start,
            ..
        }) => true,
        BotEvent::Message(msg) => {
            // TOTP codes allowed when not authorized (login attempt)
            if let Some(ref text) = msg.text
                && is_totp_code(text)
                && !state.is_authorized(user_id).await
            {
                return true;
            }
            state.is_authorized(user_id).await
        }
        _ => state.is_authorized(user_id).await,
    }
}

#[allow(dead_code)]
fn is_totp_code(text: &str) -> bool {
    text.len() == 6 && text.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
pub fn domain_resume_target(
    action: &MessageAction,
) -> Option<crate::core::types::DomainFlowSource> {
    match action {
        MessageAction::DomainReady { source, .. } => Some(*source),
        _ => None,
    }
}

#[allow(dead_code)]
async fn handle_message(msg: MessageEvent, state: &AppState) -> Result<()> {
    let action = message::handle_message(
        &*msg.adapter,
        &msg.target,
        msg.text.as_deref(),
        msg.file_id.is_some(),
        state,
    )
    .await?;

    if let MessageAction::NeedsDestruct = action
        && let Some(ref text) = msg.text
    {
        let code = text.trim();
        if is_totp_code(code) && !state.is_authorized(msg.user_id).await {
            let _ = auth::process_auth_code(
                &*msg.adapter,
                &msg.target,
                msg.user_id,
                code,
                state,
                5,
                Duration::from_secs(600),
                &[
                    Duration::from_secs(900),
                    Duration::from_secs(3600),
                    Duration::from_secs(86400),
                    Duration::from_secs(172800),
                ],
            )
            .await;
        }
    }

    let file_timeout = Duration::from_secs(180);
    if let Some(ref fid) = msg.file_id
        && state
            .take_security_file_input_status(&msg.target.0, file_timeout)
            .await
            == TimeoutStatus::Active
    {
        const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
        let content = msg.adapter.download_file(fid).await?;
        if content.len() as u64 > MAX_FILE_SIZE {
            msg.adapter
                .send_message(
                    &msg.target,
                    MessageContent {
                        text: rust_i18n::t!(
                            "bot_commands.file_too_big",
                            "0" => content.len() as u64,
                            "1" => MAX_FILE_SIZE
                        )
                        .into(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(());
        }
        let hash = hex::encode(sha2::Sha256::digest(&content));
        state.set_self_destruct_key_hash(Some(hash.clone())).await;
        if let Err(e) = crate::bootstrap::save_self_destruct_key_hash_to_config(Some(hash.clone()))
        {
            log::error!("保存安全文件雜湊失敗: {}", e);
        }
        let file_display = msg
            .file_name
            .as_ref()
            .map(|n| format!("{} | {}", n, &hash[..8]))
            .unwrap_or_else(|| hash[..8].to_string());
        msg.adapter
            .send_message(
                &msg.target,
                MessageContent {
                    text: rust_i18n::t!(
                        "bot_commands.security_file_set",
                        "0" => file_display
                    )
                    .into(),
                    markup: None,
                },
            )
            .await?;
        return Ok(());
    }

    if let MessageAction::DomainReady { source, mode } = &action {
        let source_val = *source;
        match source_val {
            DomainFlowSource::Standalone => {
                if let XhttpDeployMode::Tls { domain, cert_paths } = mode {
                    let adapter = msg.adapter.clone();
                    let target = msg.target.clone();
                    let domain = domain.clone();
                    let cert_paths = cert_paths.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handlers::xray::run_standalone_xhttp_tls(
                            domain,
                            cert_paths,
                            DomainFlowSource::Standalone,
                            adapter,
                            target,
                        )
                        .await
                        {
                            log::error!("run_standalone_xhttp_tls failed: {}", e);
                        }
                    });
                }
            }
            DomainFlowSource::OneClick => {
                if let XhttpDeployMode::Tls { domain, cert_paths } = mode {
                    let event = CallbackEvent {
                        adapter: msg.adapter.clone(),
                        target: msg.target.clone(),
                        user_id: msg.user_id.to_string(),
                        msg_id: MessageId("".into()),
                        data: "a_one_click_tls".into(),
                        callback_id: "".into(),
                        session_timeout_secs: 600,
                    };
                    let domain = domain.clone();
                    let cert_paths = cert_paths.clone();
                    tokio::spawn(async move {
                        let mode = XhttpDeployMode::Tls { domain, cert_paths };
                        if let Err(e) = handlers::ops::run_one_click(event, (), mode).await {
                            log::error!("run_one_click TLS failed: {}", e);
                        }
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod dispatch_security_file_tests {
    use super::*;

    use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use crate::shared::types::MessageEvent;

    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use std::time::Instant;

    struct TestAdapter;
    #[async_trait]
    impl BotAdapter for TestAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _t: &TargetId,
            _c: MessageContent,
        ) -> anyhow::Result<MessageId> {
            Ok(MessageId("0".into()))
        }
        async fn edit_message(
            &self,
            _t: &TargetId,
            _m: &MessageId,
            _c: MessageContent,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> anyhow::Result<()> {
            Ok(())
        }
        async fn download_file(&self, fid: &str) -> anyhow::Result<Vec<u8>> {
            Ok(fid.as_bytes().to_vec())
        }
        fn capabilities(&self) -> crate::common::PlatformCapabilities {
            crate::common::PlatformCapabilities::TELEGRAM
        }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn file_captured_when_pending_sets_hash() {
        let secret = TotpManager::generate_new_secret();
        let state = Arc::new(AppState::new(
            Some(42),
            None,
            Some(TotpManager::new(&secrecy::SecretString::from(secret)).unwrap()),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(TestAdapter),
        ));
        state.record_auth_success(42, Instant::now()).await;
        state
            .start_security_file_input("42".into(), Instant::now())
            .await;

        let msg = MessageEvent {
            adapter: Arc::new(TestAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            user_id: 42,
            text: None,
            file_id: Some("test-file".into()),
            file_name: Some("test.txt".into()),
            reply_to_text: None,
            thread_root: None,
        };
        handle_message(msg, &state).await.unwrap();
        let hash = state.self_destruct_key_hash().await;
        assert!(hash.is_some(), "hash should be set after file capture");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::state::AppState;
    use crate::app::workflows::destruct::DestructStep;
    use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use crate::shared::types::{BotCommand, BotEvent, CallbackEvent, CommandEvent, MessageEvent};

    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[derive(Default)]
    struct MockAdapter {
        pub sent: Mutex<Vec<String>>,
        pub button_data: Mutex<Vec<String>>,
        pub button_text: Mutex<Vec<String>>,
        pub callback_answers: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BotAdapter for MockAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            self.sent.lock().unwrap().push(content.text);
            Ok(MessageId("0".to_string()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            content: MessageContent,
        ) -> anyhow::Result<()> {
            self.sent.lock().unwrap().push(content.text);
            if let Some(markup) = &content.markup {
                for row in &markup.buttons {
                    for btn in row {
                        self.button_data.lock().unwrap().push(btn.data.clone());
                        self.button_text.lock().unwrap().push(btn.text.clone());
                    }
                }
            }
            Ok(())
        }
        async fn answer_callback(
            &self,
            _target: &TargetId,
            _callback_id: &str,
            text: Option<String>,
        ) -> anyhow::Result<()> {
            if let Some(t) = text {
                self.callback_answers.lock().unwrap().push(t);
            }
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
        fn capabilities(&self) -> crate::common::PlatformCapabilities {
            crate::common::PlatformCapabilities::TELEGRAM
        }
    }

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            Some(42),
            None,
            Some(
                TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter::default()),
        )
    }

    fn command_event(adapter: Arc<MockAdapter>, command: BotCommand) -> BotEvent {
        BotEvent::Command(CommandEvent {
            adapter,
            target: TargetId("123".into()),
            user_id: 42,
            command,
        })
    }

    fn callback_event(adapter: Arc<MockAdapter>, data: &str) -> BotEvent {
        BotEvent::Callback(CallbackEvent {
            adapter,
            target: TargetId("123".into()),
            user_id: "42".into(),
            msg_id: MessageId("1".into()),
            data: data.into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        })
    }

    fn message_event(adapter: Arc<MockAdapter>, target: &str, text: Option<String>) -> BotEvent {
        BotEvent::Message(MessageEvent {
            adapter,
            target: TargetId(target.into()),
            user_id: 42,
            text,
            file_id: None,
            file_name: None,
            reply_to_text: None,
            thread_root: None,
        })
    }

    #[tokio::test]
    async fn command_help_sends_help_text() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        dispatch_event(command_event(adapter.clone(), BotCommand::Help), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].is_empty());
    }

    #[tokio::test]
    async fn callback_runs_state_ops_and_handlers_dispatch() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        dispatch_event(callback_event(adapter.clone(), "m_main"), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert!(
            !sent.is_empty(),
            "authorized callback should be dispatched to handlers"
        );
    }

    #[tokio::test]
    async fn message_in_destruct_state_is_handled_by_destruct() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        state.begin_destruct("42".to_string(), Instant::now()).await;
        let totp = state.generate_current_totp().unwrap().unwrap();
        dispatch_event(message_event(adapter.clone(), "42", Some(totp)), &state)
            .await
            .unwrap();
        let snap = state.destruct_snapshot("42").await.unwrap();
        assert_eq!(snap.step, DestructStep::AwaitConfirm);
    }

    #[tokio::test]
    async fn unauthorized_callback_is_not_dispatched() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        // admin 42 but no session recorded -> not authorized
        dispatch_event(callback_event(adapter.clone(), "m_main"), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert!(
            sent.is_empty(),
            "unauthorized callback must not be dispatched (auth denied)"
        );
    }

    #[tokio::test]
    async fn totp_code_message_when_unauthorized_triggers_auth() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        // not authorized initially
        assert!(!state.is_authorized(42).await);
        let code = state.generate_current_totp().unwrap().unwrap();
        dispatch_event(message_event(adapter.clone(), "123", Some(code)), &state)
            .await
            .unwrap();
        assert!(
            state.is_authorized(42).await,
            "valid TOTP code should authorize the user"
        );
    }

    #[tokio::test]
    async fn domain_choice_buttons_are_localized_and_routable() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        dispatch_event(
            callback_event(adapter.clone(), "u_xhttp_batch_init"),
            &state,
        )
        .await
        .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_ne!(sent.last().map(String::as_str), Some("domain.prompt"));
        drop(sent);

        let buttons = adapter.button_data.lock().unwrap();
        assert!(
            buttons.contains(&"xhttp_domain_yes:standalone".to_string()),
            "should have yes button, got: {:?}",
            *buttons
        );
        assert!(
            buttons.contains(&"xhttp_domain_no:standalone".to_string()),
            "should have no button, got: {:?}",
            *buttons
        );
        drop(buttons);

        let button_text = adapter.button_text.lock().unwrap();
        assert!(!button_text.contains(&"domain.yes".to_string()));
        assert!(!button_text.contains(&"domain.no".to_string()));
    }

    #[tokio::test]
    async fn xhttp_domain_yes_routes_to_xray_handler() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        dispatch_event(
            callback_event(adapter.clone(), "xhttp_domain_yes:standalone"),
            &state,
        )
        .await
        .unwrap();
        // xhttp_domain_yes starts domain input flow, answer_callback is called
        let sent = adapter.sent.lock().unwrap();
        assert!(
            !sent.is_empty(),
            "domain yes callback should produce a response"
        );
    }

    #[tokio::test]
    async fn xhttp_domain_provider_routes_to_xray_handler() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        // Start domain flow and transition to AwaitProvider state
        state
            .start_domain_input(
                "123".into(),
                crate::core::types::DomainFlowSource::Standalone,
                Instant::now(),
            )
            .await;
        state
            .transition_domain_input(
                "123",
                crate::core::types::DomainInputStep::AwaitDomain,
                crate::core::types::DomainInputStep::AwaitProvider,
                None,
            )
            .await;
        dispatch_event(
            callback_event(adapter.clone(), "xhttp_domain_provider:cloudflare"),
            &state,
        )
        .await
        .unwrap();
        let domain_state = state.domain_input_snapshot("123").await.unwrap();
        assert_eq!(
            domain_state.step,
            crate::core::types::DomainInputStep::AwaitCredentials(
                crate::core::types::DnsProvider::Cloudflare
            )
        );
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(
            sent.as_slice(),
            &[
                crate::shared::handlers::message::provider_credential_guidance(
                    crate::core::types::DnsProvider::Cloudflare
                )
            ]
        );
    }

    #[tokio::test]
    async fn stale_domain_provider_callback_is_rejected() {
        let prev_lang = crate::core::i18n::current_lang();
        crate::core::i18n::set_lang(crate::core::i18n::Lang::En);
        let adapter = Arc::new(MockAdapter::default());
        let state = Arc::new(make_state());
        state.record_auth_success(42, Instant::now()).await;
        state
            .start_domain_input(
                "123".into(),
                crate::core::types::DomainFlowSource::Standalone,
                Instant::now(),
            )
            .await;
        state
            .transition_domain_input(
                "123",
                crate::core::types::DomainInputStep::AwaitDomain,
                crate::core::types::DomainInputStep::AwaitProvider,
                None,
            )
            .await;

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let cloudflare = async {
            barrier.wait().await;
            dispatch_event(
                callback_event(adapter.clone(), "xhttp_domain_provider:cloudflare"),
                &state,
            )
            .await
        };
        let route53 = async {
            barrier.wait().await;
            dispatch_event(
                callback_event(adapter.clone(), "xhttp_domain_provider:route53"),
                &state,
            )
            .await
        };

        let (cloudflare_result, route53_result) = tokio::join!(cloudflare, route53);
        cloudflare_result.unwrap();
        route53_result.unwrap();

        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "only the winning provider gets a prompt");
        let expected = [
            crate::shared::handlers::message::provider_credential_guidance(
                crate::core::types::DnsProvider::Cloudflare,
            ),
            crate::shared::handlers::message::provider_credential_guidance(
                crate::core::types::DnsProvider::Route53,
            ),
        ];
        assert!(expected.contains(&sent[0]));
        drop(sent);
        let answers = adapter.callback_answers.lock().unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0], rust_i18n::t!("domain.flow_expired"));
        crate::core::i18n::set_lang(prev_lang);
    }

    #[tokio::test]
    async fn domain_ready_dispatches_by_original_source() {
        use crate::core::security::acme::{CertPaths, XhttpDeployMode};
        use crate::core::types::DomainFlowSource;
        use crate::shared::handlers::message::MessageAction;
        use std::path::PathBuf;

        fn tls_mode(domain: &str) -> XhttpDeployMode {
            XhttpDeployMode::Tls {
                domain: domain.to_string(),
                cert_paths: CertPaths {
                    fullchain: PathBuf::from("/fake/fullchain"),
                    privkey: PathBuf::from("/fake/privkey"),
                },
            }
        }

        let action = MessageAction::DomainReady {
            source: DomainFlowSource::Standalone,
            mode: tls_mode("example.com"),
        };
        assert_eq!(
            domain_resume_target(&action),
            Some(DomainFlowSource::Standalone)
        );

        let action_oneclick = MessageAction::DomainReady {
            source: DomainFlowSource::OneClick,
            mode: tls_mode("example.com"),
        };
        assert_eq!(
            domain_resume_target(&action_oneclick),
            Some(DomainFlowSource::OneClick)
        );
    }
}
