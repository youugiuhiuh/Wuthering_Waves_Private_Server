use std::time::Duration;

use anyhow::Result;
use sha2::Digest;

use crate::app::auth;
use crate::app::state::AppState;
use crate::common::{MessageContent, MessageId, TargetId};
use crate::core::security::acme::XhttpDeployMode;
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
            if try_confirm_security_code(&msg, state).await? {
                return Ok(());
            }
            handle_message(msg, state).await?;
        }
        BotEvent::Callback(mut cb) => {
            // State operations (lang, set_timeout, warp input). The lang branch
            // may return a redirect (e.g. re-show main menu).
            if let Some(next) = state_ops::intercept(&cb, state).await {
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
    let user_id = event.user_id();

    // 6 位纯数字消息是**登录尝试**，必须对任何身份放行到 `auth::process_auth_code`。
    // 这是 SimpleX 管理员自愈的前提：新 contactId（删联系人重连 / 换设备）在重钉之前
    // 一定不是管理员，若在这里就挡掉，它就永远发不出那个能证明自己的码。
    // 安全性由「全局限流 + TOTP 本身」承担，而不是由这道白名单承担。
    let is_login_attempt = matches!(
        event,
        BotEvent::Message(msg) if msg.text.as_deref().is_some_and(is_totp_code)
    ) && !state.is_authorized(user_id).await;

    if !state.is_admin_user(user_id) && !is_login_attempt {
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

/// 安全码最少数字位数；低于此值的纯数字消息（如 6 位 TOTP）不算码。
const MIN_CODE_DIGITS: usize = 12;

/// 归一化安全码：仅保留 ASCII 数字，去掉空格/换行/制表符等一切非数字字符。
fn normalize_security_code(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// 是否「疑似安全码」：原文只含数字与空白，且去空白后数字位数 >= `MIN_CODE_DIGITS`。
fn looks_like_security_code(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_whitespace())
        && s.chars().filter(char::is_ascii_digit).count() >= MIN_CODE_DIGITS
}

/// 处理用户回贴的 SimpleX 安全码。
///
/// 命中判据：未验证 + TG/SimpleX 管理员都在 + 文本形如安全码。命中后现取当前
/// 安全码比对：一致则置位并落盘、回复成功；不一致则回复失败且不落盘。两条路都
/// 返回 `true`（短路后续普通处理）。不命中或取码失败（fail-open）返回 `false`。
async fn try_confirm_security_code(msg: &MessageEvent, state: &AppState) -> Result<bool> {
    if state.simplex_code_verified() {
        return Ok(false);
    }
    let Some(sx_admin) = state.simplex_admin_id() else {
        return Ok(false);
    };
    let Some(tg_admin) = state.admin_id() else {
        return Ok(false);
    };
    let Some(text) = msg.text.as_deref() else {
        return Ok(false);
    };
    if !looks_like_security_code(text) {
        return Ok(false);
    }

    // 现取码；失败不阻塞普通消息处理（fail-open）。
    let fetched = match state.adapter.contact_security_code(sx_admin).await {
        Ok(code) => code,
        Err(e) => {
            log::warn!("获取 SimpleX 安全码失败（不影响普通消息处理）: {e}");
            return Ok(false);
        }
    };

    let target = TargetId(tg_admin.to_string());
    // 强制 primary(TG)：安全码回复若走 `send_message`，可能被敏感分流改投 SimpleX。
    if normalize_security_code(text) == normalize_security_code(&fetched) {
        // 先更新内存态：即使落盘失败，本次运行也已可用。
        state.set_simplex_code_verified_for(Some(sx_admin));
        if let Err(e) = crate::bootstrap::set_simplex_code_verified(
            &crate::bootstrap::config_dir(),
            Some(sx_admin),
        ) {
            log::error!("保存安全码已验证状态失败（内存态保留）: {e}");
        }
        if let Err(e) = state
            .adapter
            .send_message_primary(
                &target,
                MessageContent {
                    text: rust_i18n::t!("simplex.code_verified").to_string(),
                    markup: None,
                },
            )
            .await
        {
            log::warn!("回复安全码已验证失败: {e}");
        }
    } else {
        log::warn!("用户回贴的安全码与现取不一致（contactId={sx_admin}）");
        if let Err(e) = state
            .adapter
            .send_message_primary(
                &target,
                MessageContent {
                    text: rust_i18n::t!("simplex.code_mismatch").to_string(),
                    markup: None,
                },
            )
            .await
        {
            log::warn!("回复安全码不一致失败: {e}");
        }
    }
    Ok(true)
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

    use crate::app::state::{AppState, DestructStep};
    use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use crate::shared::types::{BotCommand, BotEvent, CallbackEvent, CommandEvent, MessageEvent};

    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[derive(Default)]
    struct MockAdapter {
        pub sent: Mutex<Vec<String>>,
        pub button_data: Mutex<Vec<String>>,
        pub button_text: Mutex<Vec<String>>,
        pub callback_answers: Mutex<Vec<String>>,
        /// 经 `send_message_primary` 发出的文本（与 `sent` 同步记录，额外单独留痕）。
        pub primary_sent: Mutex<Vec<String>>,
        /// `contact_security_code` 的返回；`None` 表示取码失败。
        pub security_code: Mutex<Option<String>>,
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
        async fn contact_security_code(&self, _contact_id: i64) -> anyhow::Result<String> {
            self.security_code
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("未配置测试安全码（模拟取码失败）"))
        }
        async fn send_message_primary(
            &self,
            target: &TargetId,
            content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            self.primary_sent.lock().unwrap().push(content.text.clone());
            self.send_message(target, content).await
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

    /// 与 `message_event` 相同，但身份显式 —— 既有 `message_event` 把 `user_id` 硬编码为 42，
    /// 而那正是 `make_state()` 的管理员，用它无法测到「非管理员」路径。
    fn message_event_from(
        adapter: Arc<MockAdapter>,
        user_id: i64,
        text: Option<String>,
    ) -> BotEvent {
        BotEvent::Message(MessageEvent {
            adapter,
            target: TargetId(user_id.to_string()),
            user_id,
            text,
            file_id: None,
            file_name: None,
            reply_to_text: None,
            thread_root: None,
        })
    }

    /// 未授权身份发 6 位码必须能通过 check_auth（登录尝试），否则重钉路径不可达。
    #[tokio::test]
    async fn totp_code_from_non_admin_passes_check_auth() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99, // 不是 make_state() 配的管理员（42）
            Some("123456".to_string()),
        );
        assert!(
            check_auth(&event, &state).await,
            "6 位码是登录尝试，必须放行到 process_auth_code；否则新 contactId 永远无法自愈"
        );
    }

    /// 非 6 位码的普通消息来自非管理员时必须仍然被挡。
    #[tokio::test]
    async fn non_code_message_from_non_admin_is_still_rejected() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99,
            Some("hello".to_string()),
        );
        assert!(!check_auth(&event, &state).await);
    }

    /// 6 位但非全数字（如 "12345a"）不算码，仍须被挡。
    #[tokio::test]
    async fn non_numeric_six_chars_from_non_admin_is_rejected() {
        let state = make_state();
        let event = message_event_from(
            Arc::new(MockAdapter::default()),
            99,
            Some("12345a".to_string()),
        );
        assert!(!check_auth(&event, &state).await);
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
        let totp = state.generate_current_totp().unwrap();
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
        let code = state.generate_current_totp().unwrap();
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

    /// Mutates the process-global locale, so it must not run concurrently
    /// with any other test that sets or asserts the locale. nextest's
    /// per-process isolation hides the race; plain `cargo test` does not.
    #[serial]
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

    #[test]
    fn normalize_security_code_strips_whitespace_keeps_digits() {
        assert_eq!(
            normalize_security_code("54440 24092\n64994"),
            "544402409264994"
        );
        assert_eq!(normalize_security_code("07 533\t47951"), "0753347951");
        assert_eq!(normalize_security_code("a1 b2\nc3"), "123");
        assert_eq!(normalize_security_code(""), "");
    }

    #[test]
    fn looks_like_security_code_accepts_pure_digits_spanning_lines() {
        assert!(looks_like_security_code("54440 24092\n64994"));
        assert!(looks_like_security_code("123456789012"));
        assert!(looks_like_security_code("544402409264994"));
    }

    #[test]
    fn looks_like_security_code_rejects_non_code_inputs() {
        assert!(!looks_like_security_code(""));
        assert!(!looks_like_security_code("/menu"));
        assert!(!looks_like_security_code("123456")); // 6-digit TOTP
        assert!(!looks_like_security_code("12345678901")); // 11 digits
        assert!(!looks_like_security_code("12345 6789a"));
        assert!(!looks_like_security_code("example.com"));
    }

    // --- S5: 入站安全码确认 ---

    /// TG 管理员 42 + SimpleX 联系人 7 的组合态（安全码确认要求两者都存在）。
    fn make_tg_simplex_state(adapter: Arc<MockAdapter>) -> AppState {
        AppState::new(
            Some(42),
            Some(7),
            Some(
                TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            adapter,
        )
    }

    /// 纯 `--simplex`：无 TG 管理员（`admin_id == None`）。
    fn make_simplex_only_state(adapter: Arc<MockAdapter>) -> AppState {
        AppState::new(
            None,
            Some(7),
            Some(
                TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            adapter,
        )
    }

    fn set_config_dir(dir: &tempfile::TempDir) {
        // SAFETY: `#[serial]` 保证没有并发读该环境变量的测试；nextest 每个测试独立进程。
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
        }
    }

    #[serial]
    #[tokio::test]
    async fn matching_security_code_marks_verified_persists_and_swallows_message() {
        let dir = tempfile::TempDir::new().unwrap();
        set_config_dir(&dir);
        let adapter = Arc::new(MockAdapter::default());
        // 5000 位数字：若未短路而落到 handle_message，必然触发 message.input_too_long。
        let code = "5".repeat(5000);
        *adapter.security_code.lock().unwrap() = Some(code.clone());
        let state = make_tg_simplex_state(adapter.clone());
        state.record_auth_success(42, Instant::now()).await;

        dispatch_event(message_event(adapter.clone(), "42", Some(code)), &state)
            .await
            .unwrap();

        assert!(state.simplex_code_verified(), "一致后内存态应置为已验证");
        assert_eq!(
            crate::bootstrap::BotSettings::load().simplex_code_verified_for,
            Some(7),
            "一致后应把 contactId 落盘"
        );
        let expected = rust_i18n::t!("simplex.code_verified").to_string();
        let primary = adapter.primary_sent.lock().unwrap();
        assert_eq!(primary.len(), 1, "只应回复一条安全码确认消息");
        assert_eq!(
            primary[0], expected,
            "回复应走 send_message_primary 且为本地化确认文案，got: {:?}",
            *primary
        );
        drop(primary);
        // 短路证据：handle_message 若被调用，5000 位输入会再追加一条 input_too_long。
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(
            sent.len(),
            1,
            "命中后必须短路，只应有确认回复一条，got: {:?}",
            *sent
        );
        assert_eq!(sent[0], expected);
    }

    /// 落盘失败必须只告警、保留内存态并照常回复（spec §4.1 fail-open）。
    #[serial]
    #[tokio::test]
    async fn persist_failure_keeps_memory_and_replies() {
        // config 目录指向一个文件的子路径：create_dir_all 必然失败。
        let file = tempfile::NamedTempFile::new().unwrap();
        let bad = format!("{}/sub", file.path().display());
        // SAFETY: `#[serial]` 保证没有并发读写该环境变量的测试；nextest 每个测试独立进程。
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", &bad);
        }
        let adapter = Arc::new(MockAdapter::default());
        let code = "123456789012".to_string();
        *adapter.security_code.lock().unwrap() = Some(code.clone());
        let state = make_tg_simplex_state(adapter.clone());
        state.record_auth_success(42, Instant::now()).await;

        dispatch_event(message_event(adapter.clone(), "42", Some(code)), &state)
            .await
            .unwrap();

        assert!(state.simplex_code_verified(), "落盘失败也应保留内存态");
        let primary = adapter.primary_sent.lock().unwrap();
        assert_eq!(primary.len(), 1, "落盘失败也应回复确认");
        assert_eq!(
            primary[0],
            rust_i18n::t!("simplex.code_verified").to_string()
        );
    }

    #[serial]
    #[tokio::test]
    async fn mismatching_security_code_replies_and_does_not_persist() {
        let dir = tempfile::TempDir::new().unwrap();
        set_config_dir(&dir);
        let adapter = Arc::new(MockAdapter::default());
        *adapter.security_code.lock().unwrap() = Some("9999888877776666".to_string());
        let state = make_tg_simplex_state(adapter.clone());
        state.record_auth_success(42, Instant::now()).await;

        dispatch_event(
            message_event(adapter.clone(), "42", Some("123456789012".to_string())),
            &state,
        )
        .await
        .unwrap();

        assert!(!state.simplex_code_verified(), "不一致不得置为已验证");
        assert_eq!(
            crate::bootstrap::BotSettings::load().simplex_code_verified_for,
            None,
            "不一致不得落盘"
        );
        let primary = adapter.primary_sent.lock().unwrap();
        assert_eq!(primary.len(), 1);
        assert_eq!(
            primary[0],
            rust_i18n::t!("simplex.code_mismatch").to_string(),
            "回复应为本地化不一致文案，got: {:?}",
            *primary
        );
    }

    #[serial]
    #[tokio::test]
    async fn ordinary_messages_do_not_enter_confirmation() {
        let dir = tempfile::TempDir::new().unwrap();
        set_config_dir(&dir);
        // 命令、域名、6 位 TOTP、含字母的类码输入，全部不走确认路径（行为逐条不变）。
        for text in ["/menu", "example.com", "123456", "12345 6789a"] {
            let adapter = Arc::new(MockAdapter::default());
            *adapter.security_code.lock().unwrap() = Some("123456789012".to_string());
            let state = make_tg_simplex_state(adapter.clone());
            state.record_auth_success(42, Instant::now()).await;

            dispatch_event(
                message_event(adapter.clone(), "42", Some(text.to_string())),
                &state,
            )
            .await
            .unwrap();

            assert!(
                adapter.primary_sent.lock().unwrap().is_empty(),
                "普通消息 {text:?} 不应触发安全码确认"
            );
            assert!(!state.simplex_code_verified());
        }
    }

    #[serial]
    #[tokio::test]
    async fn simplex_only_deployment_skips_confirmation() {
        let dir = tempfile::TempDir::new().unwrap();
        set_config_dir(&dir);
        let adapter = Arc::new(MockAdapter::default());
        *adapter.security_code.lock().unwrap() = Some("123456789012".to_string());
        let state = make_simplex_only_state(adapter.clone());
        state.record_auth_success(7, Instant::now()).await;

        dispatch_event(
            message_event_from(adapter.clone(), 7, Some("123456789012".to_string())),
            &state,
        )
        .await
        .unwrap();

        assert!(
            adapter.primary_sent.lock().unwrap().is_empty(),
            "纯 --simplex（admin_id == None）不得进入确认路径"
        );
        assert!(!state.simplex_code_verified());
    }

    #[serial]
    #[tokio::test]
    async fn security_code_fetch_failure_fails_open() {
        let dir = tempfile::TempDir::new().unwrap();
        set_config_dir(&dir);
        let adapter = Arc::new(MockAdapter::default());
        // 未配置 security_code → contact_security_code 返回 Err。
        let state = make_tg_simplex_state(adapter.clone());
        state.record_auth_success(42, Instant::now()).await;

        dispatch_event(
            message_event(adapter.clone(), "42", Some("123456789012".to_string())),
            &state,
        )
        .await
        .unwrap();

        assert!(
            adapter.primary_sent.lock().unwrap().is_empty(),
            "取码失败应 fail-open，不回复也不打断普通处理"
        );
        assert!(!state.simplex_code_verified());
    }
}
