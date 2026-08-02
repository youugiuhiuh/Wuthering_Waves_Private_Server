use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use aegis::app::workflows::certificate::CertificateWorkflow;
use aegis::app::workflows::destruct::{DestructState, DestructStep, DestructWorkflow};
use aegis::app::workflows::schedule::{ScheduleFlow, ScheduleWorkflow};
use aegis::app::workflows::warp::{WarpFlow, WarpWorkflow};
use aegis::common::BotAdapter;
use aegis::core::i18n::Lang;
use aegis::core::security::self_destruct::SelfDestructExecutor;
use aegis::core::totp::TotpManager;
use aegis::core::types::{DomainFlowSource, DomainInputState, DomainInputStep};
use aegis::shared::handlers::message::MessageState;
use aegis::shared::types::TimeoutStatus;

const RECENT_AUTH_WINDOW_SECS: u64 = 5 * 60;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailureOutcome {
    Invalid { attempts: u32, max_attempts: u32 },
    Locked { duration: Duration },
}

#[derive(Debug, Clone)]
pub struct FailedRecord {
    pub count: u32,
    pub first_fail: Instant,
    pub cooldown_until: Option<Instant>,
    pub lock_level: usize,
}

pub struct AppState {
    #[allow(dead_code)]
    pub adapter: Arc<dyn BotAdapter>,
    admin_id: Option<i64>,
    discord_admin_id: Option<i64>,
    totp_manager: Option<TotpManager>,
    self_destruct_executor: Arc<dyn SelfDestructExecutor>,
    sessions: Mutex<HashMap<i64, Instant>>,
    failed_attempts: Mutex<HashMap<i64, FailedRecord>>,
    pending_destructs: DestructWorkflow,
    self_destruct_key_hash: Mutex<Option<String>>,
    warp_workflow: WarpWorkflow,
    schedule_workflow: ScheduleWorkflow,
    pending_security_file: Mutex<HashMap<String, Instant>>,
    certificate_workflow: CertificateWorkflow,
    session_timeout_secs: Mutex<u64>,
    lang: Mutex<Lang>,
    lang_configured: Mutex<bool>,
}

impl AppState {
    pub fn new(
        admin_id: Option<i64>,
        discord_admin_id: Option<i64>,
        totp_manager: Option<TotpManager>,
        self_destruct_executor: Arc<dyn SelfDestructExecutor>,
        self_destruct_key_hash: Option<String>,
        session_timeout_secs: u64,
        adapter: Arc<dyn BotAdapter>,
    ) -> Self {
        Self {
            adapter,
            admin_id,
            discord_admin_id,
            totp_manager,
            self_destruct_executor,
            sessions: Mutex::new(HashMap::new()),
            failed_attempts: Mutex::new(HashMap::new()),
            pending_destructs: DestructWorkflow::default(),
            self_destruct_key_hash: Mutex::new(self_destruct_key_hash),
            warp_workflow: WarpWorkflow::default(),
            schedule_workflow: ScheduleWorkflow::default(),
            pending_security_file: Mutex::new(HashMap::new()),
            certificate_workflow: CertificateWorkflow::default(),
            session_timeout_secs: Mutex::new(session_timeout_secs),
            lang: Mutex::new(Lang::Zh),
            lang_configured: Mutex::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn admin_id(&self) -> Option<i64> {
        self.admin_id
    }

    pub fn is_admin_user(&self, user_id: i64) -> bool {
        user_id == self.admin_id.unwrap_or(0) || self.discord_admin_id == Some(user_id)
    }

    pub fn verify_totp(&self, code: &str) -> bool {
        self.totp_manager
            .as_ref()
            .map(|m| m.verify(code))
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn generate_current_totp(&self) -> Option<Result<String, std::time::SystemTimeError>> {
        self.totp_manager.as_ref().map(|m| m.generate_current())
    }

    pub async fn is_authorized(&self, user_id: i64) -> bool {
        if !self.is_admin_user(user_id) {
            return false;
        }

        let timeout = self.session_timeout_secs().await;
        let sessions = self.sessions.lock().await;
        sessions
            .get(&user_id)
            .map(|t| is_session_valid(t, timeout))
            .unwrap_or(false)
    }

    pub async fn is_recently_authenticated(&self, user_id: i64) -> bool {
        if !self.is_admin_user(user_id) {
            return false;
        }

        let sessions = self.sessions.lock().await;
        sessions
            .get(&user_id)
            .map(|t| is_session_valid(t, RECENT_AUTH_WINDOW_SECS))
            .unwrap_or(false)
    }

    pub async fn session_timeout_secs(&self) -> u64 {
        *self.session_timeout_secs.lock().await
    }

    pub async fn set_session_timeout_secs(&self, secs: u64) {
        *self.session_timeout_secs.lock().await = secs;
    }

    #[allow(dead_code)]
    pub async fn lang(&self) -> Lang {
        *self.lang.lock().await
    }

    pub async fn set_lang(&self, lang: Lang) {
        *self.lang.lock().await = lang;
    }

    pub async fn is_lang_configured(&self) -> bool {
        *self.lang_configured.lock().await
    }

    pub async fn mark_lang_configured(&self) {
        *self.lang_configured.lock().await = true;
    }

    pub fn self_destruct_executor(&self) -> Arc<dyn SelfDestructExecutor> {
        self.self_destruct_executor.clone()
    }

    pub async fn set_self_destruct_key_hash(&self, hash: Option<String>) {
        *self.self_destruct_key_hash.lock().await = hash;
    }

    pub async fn self_destruct_key_hash(&self) -> Option<String> {
        self.self_destruct_key_hash.lock().await.clone()
    }

    pub async fn record_auth_success(&self, user_id: i64, now: Instant) -> u64 {
        self.sessions.lock().await.insert(user_id, now);
        self.failed_attempts.lock().await.remove(&user_id);
        self.session_timeout_secs().await
    }

    pub async fn record_auth_failure(
        &self,
        user_id: i64,
        now: Instant,
        max_attempts: u32,
        failure_window: Duration,
        lockout_durations: &[Duration],
    ) -> AuthFailureOutcome {
        let mut fails = self.failed_attempts.lock().await;
        let rec = fails.entry(user_id).or_insert(FailedRecord {
            count: 0,
            first_fail: now,
            cooldown_until: None,
            lock_level: 0,
        });

        if now.duration_since(rec.first_fail) > failure_window {
            rec.count = 0;
            rec.first_fail = now;
            rec.cooldown_until = None;
        }

        rec.count += 1;
        rec.first_fail = rec.first_fail.min(now);

        if rec.count >= max_attempts {
            let duration = lockout_durations
                .get(rec.lock_level)
                .copied()
                .or_else(|| lockout_durations.last().copied())
                .unwrap_or(Duration::from_secs(3600));

            rec.cooldown_until = Some(now + duration);
            rec.count = 0;
            rec.first_fail = now;

            if rec.lock_level < lockout_durations.len() - 1 {
                rec.lock_level += 1;
            }

            AuthFailureOutcome::Locked { duration }
        } else {
            AuthFailureOutcome::Invalid {
                attempts: rec.count,
                max_attempts,
            }
        }
    }

    pub async fn auth_cooldown_remaining(&self, user_id: i64, now: Instant) -> Option<Duration> {
        let mut fails = self.failed_attempts.lock().await;
        let rec = fails.get_mut(&user_id)?;
        let until = rec.cooldown_until?;
        if until > now {
            Some(until - now)
        } else {
            rec.cooldown_until = None;
            None
        }
    }

    pub async fn begin_destruct(&self, chat_id: String, now: Instant) {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id) else {
            return;
        };
        self.pending_destructs.begin(conversation, now);
    }

    pub async fn cancel_destruct(&self, chat_id: &str) -> bool {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return false;
        };
        self.pending_destructs.cancel(&conversation)
    }

    pub async fn destruct_snapshot(&self, chat_id: &str) -> Option<DestructState> {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return None;
        };
        self.pending_destructs.snapshot(&conversation)
    }

    pub async fn touch_destruct(
        &self,
        chat_id: &str,
        now: Instant,
        timeout: Duration,
    ) -> TimeoutStatus {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return TimeoutStatus::NotTracked;
        };
        self.pending_destructs.touch(&conversation, now, timeout)
    }

    pub async fn advance_destruct_step(
        &self,
        chat_id: &str,
        expected: DestructStep,
        next: DestructStep,
        now: Instant,
    ) -> bool {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return false;
        };
        self.pending_destructs
            .advance_step(&conversation, expected, next, now)
    }

    pub async fn confirm_first_destruct_totp(
        &self,
        chat_id: &str,
        code: &str,
        now: Instant,
    ) -> bool {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return false;
        };
        self.pending_destructs
            .confirm_first_totp(&conversation, code, now)
    }

    pub async fn confirm_second_destruct_totp(
        &self,
        chat_id: &str,
        code: &str,
        now: Instant,
    ) -> Result<bool, String> {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return Ok(false);
        };
        self.pending_destructs
            .confirm_second_totp(&conversation, code, now)
    }

    pub async fn mark_destruct_file_verified(&self, chat_id: &str, now: Instant) -> bool {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return false;
        };
        self.pending_destructs
            .mark_file_verified(&conversation, now)
    }

    pub async fn with_destruct<R>(
        &self,
        chat_id: &str,
        f: impl FnOnce(&mut DestructState) -> R,
    ) -> Option<R> {
        let Ok(conversation) = aegis::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return None;
        };
        self.pending_destructs.with_state(&conversation, f)
    }

    pub async fn start_warp_input(&self, chat_id: String, now: Instant) {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id) else {
            return;
        };
        self.warp_workflow.start(conversation, now);
    }

    pub async fn warp_flow(&self, chat_id: &str, timeout: Duration) -> WarpFlow {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return WarpFlow::Continue;
        };
        self.warp_workflow.take(&conversation, timeout)
    }

    pub async fn schedule_flow(&self, chat_id: &str, timeout: Duration) -> ScheduleFlow {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return ScheduleFlow::Continue;
        };
        self.schedule_workflow.route(&conversation, timeout)
    }

    pub async fn start_security_file_input(&self, chat_id: String, now: Instant) {
        self.pending_security_file.lock().await.insert(chat_id, now);
    }

    pub async fn take_security_file_input_status(
        &self,
        chat_id: &str,
        timeout: Duration,
    ) -> TimeoutStatus {
        let mut map = self.pending_security_file.lock().await;
        match map.remove(chat_id) {
            Some(started) if started.elapsed() < timeout => TimeoutStatus::Active,
            Some(_) => TimeoutStatus::Expired,
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn start_domain_input(
        &self,
        chat_id: String,
        source: DomainFlowSource,
        now: Instant,
    ) {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id) else {
            return;
        };
        self.certificate_workflow.start(conversation, source, now);
    }

    pub async fn domain_input_snapshot(&self, chat_id: &str) -> Option<DomainInputState> {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return None;
        };
        self.certificate_workflow.snapshot(&conversation)
    }

    pub async fn transition_domain_input(
        &self,
        chat_id: &str,
        expected: DomainInputStep,
        next: DomainInputStep,
        domain: Option<String>,
    ) -> bool {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return false;
        };
        self.certificate_workflow
            .transition(&conversation, expected, next, domain)
    }

    pub async fn take_domain_input(&self, chat_id: &str) -> Option<DomainInputState> {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return None;
        };
        self.certificate_workflow.take(&conversation)
    }

    pub async fn domain_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        let Ok(conversation) = crate::app::interaction::ConversationId::new(chat_id.to_string())
        else {
            return TimeoutStatus::NotTracked;
        };
        self.certificate_workflow
            .timeout_status(&conversation, timeout)
    }
}

#[async_trait]
impl MessageState for AppState {
    async fn schedule_flow(&self, chat_id: &str, timeout: Duration) -> ScheduleFlow {
        self.schedule_flow(chat_id, timeout).await
    }

    async fn warp_flow(&self, chat_id: &str, timeout: Duration) -> WarpFlow {
        self.warp_flow(chat_id, timeout).await
    }

    async fn start_domain_input(
        &self,
        chat_id: String,
        source: DomainFlowSource,
        now: std::time::Instant,
    ) {
        self.start_domain_input(chat_id, source, now).await
    }

    async fn domain_input_snapshot(&self, chat_id: &str) -> Option<DomainInputState> {
        self.domain_input_snapshot(chat_id).await
    }

    async fn transition_domain_input(
        &self,
        chat_id: &str,
        expected: DomainInputStep,
        next: DomainInputStep,
        domain: Option<String>,
    ) -> bool {
        self.transition_domain_input(chat_id, expected, next, domain)
            .await
    }

    async fn take_domain_input(&self, chat_id: &str) -> Option<DomainInputState> {
        self.take_domain_input(chat_id).await
    }

    async fn domain_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        self.domain_timeout_status(chat_id, timeout).await
    }
}

fn is_session_valid(session_time: &Instant, timeout_secs: u64) -> bool {
    session_time.elapsed() < Duration::from_secs(timeout_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::common::{MessageContent, MessageId, Platform, TargetId};
    use aegis::core::types::{DomainFlowSource, DomainInputStep};
    use anyhow::Result;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct MockAdapter;

    #[async_trait]
    impl BotAdapter for MockAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> Result<MessageId> {
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
        async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> aegis::common::PlatformCapabilities {
            aegis::common::PlatformCapabilities::TELEGRAM
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            Some(42),
            None,
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        )
    }

    #[tokio::test]
    async fn authorization_success_creates_valid_session() {
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        assert!(state.is_authorized(42).await);
    }

    #[tokio::test]
    async fn repeated_auth_failures_trigger_lock() {
        let state = make_state();
        let now = Instant::now();
        let lockouts = [Duration::from_secs(1)];
        let mut outcome = None;
        for _ in 0..5 {
            outcome = Some(
                state
                    .record_auth_failure(42, now, 5, Duration::from_secs(600), &lockouts)
                    .await,
            );
        }
        assert_eq!(
            outcome,
            Some(AuthFailureOutcome::Locked {
                duration: Duration::from_secs(1),
            })
        );
    }

    #[tokio::test]
    async fn destruct_step_transitions_validate_expected_flow() {
        let state = make_state();
        let chat_id = "1".to_string();
        let now = Instant::now();
        state.begin_destruct(chat_id.clone(), now).await;
        assert!(
            state
                .confirm_first_destruct_totp(&chat_id, "111111", now)
                .await
        );
        assert!(
            state
                .advance_destruct_step(
                    &chat_id,
                    DestructStep::AwaitConfirm,
                    DestructStep::AwaitSecondTotp,
                    now,
                )
                .await
        );
        assert!(
            state
                .confirm_second_destruct_totp(&chat_id, "222222", now)
                .await
                .unwrap()
        );
        assert!(state.mark_destruct_file_verified(&chat_id, now).await);
    }

    #[tokio::test]
    async fn destruct_timeout_is_detected() {
        let state = make_state();
        let chat_id = "2".to_string();
        state
            .begin_destruct(chat_id.clone(), Instant::now() - Duration::from_secs(61))
            .await;
        assert_eq!(
            state
                .touch_destruct(&chat_id, Instant::now(), Duration::from_secs(60))
                .await,
            TimeoutStatus::Expired
        );
    }

    #[tokio::test]
    async fn is_authorized_returns_false_for_non_admin() {
        let state = make_state();
        assert!(!state.is_authorized(999).await);
    }

    #[tokio::test]
    async fn is_authorized_returns_false_for_expired_session() {
        let state = make_state();
        let past = Instant::now() - Duration::from_secs(601);
        state.record_auth_success(42, past).await;
        assert!(!state.is_authorized(42).await);
    }

    #[tokio::test]
    async fn auth_cooldown_remaining_returns_none_when_not_locked() {
        let state = make_state();
        let remaining = state.auth_cooldown_remaining(42, Instant::now()).await;
        assert!(remaining.is_none());
    }

    #[tokio::test]
    async fn auth_cooldown_remaining_returns_duration_when_locked() {
        let state = make_state();
        let now = Instant::now();
        let lockouts = [Duration::from_secs(60)];
        for _ in 0..5 {
            state
                .record_auth_failure(42, now, 5, Duration::from_secs(600), &lockouts)
                .await;
        }
        let remaining = state.auth_cooldown_remaining(42, now).await;
        assert!(remaining.is_some());
        assert!(remaining.unwrap().as_secs() <= 60);
    }

    #[tokio::test]
    async fn schedule_flow_returns_continue_for_unknown_chat() {
        let state = make_state();
        let flow = state.schedule_flow("123", Duration::from_secs(60)).await;
        assert_eq!(flow, ScheduleFlow::Continue);
    }

    #[tokio::test]
    async fn cancel_destruct_returns_true_when_exists() {
        let state = make_state();
        let chat_id = "5".to_string();
        state.begin_destruct(chat_id.clone(), Instant::now()).await;
        assert!(state.cancel_destruct(&chat_id).await);
    }

    #[tokio::test]
    async fn cancel_destruct_returns_false_when_not_exists() {
        let state = make_state();
        assert!(!state.cancel_destruct("999").await);
    }

    #[tokio::test]
    async fn warp_input_timeout_tracking() {
        let state = make_state();
        let chat_id = "200".to_string();
        state
            .start_warp_input(chat_id.clone(), Instant::now())
            .await;
        let status = state.warp_flow(&chat_id, Duration::from_secs(60)).await;
        assert_eq!(status, WarpFlow::Waiting);
    }

    #[tokio::test]
    async fn auth_failure_window_resets_count() {
        let state = make_state();
        let now = Instant::now();
        let lockouts = [Duration::from_secs(60)];
        state
            .record_auth_failure(42, now, 5, Duration::from_secs(600), &lockouts)
            .await;
        state
            .record_auth_failure(42, now, 5, Duration::from_secs(600), &lockouts)
            .await;
        let far_future = now + Duration::from_secs(601);
        let outcome = state
            .record_auth_failure(42, far_future, 5, Duration::from_secs(600), &lockouts)
            .await;
        if let AuthFailureOutcome::Invalid { attempts, .. } = outcome {
            assert_eq!(attempts, 1);
        }
    }

    #[tokio::test]
    async fn session_timeout_set_and_get() {
        let state = make_state();
        state.set_session_timeout_secs(3600).await;
        assert_eq!(state.session_timeout_secs().await, 3600);
    }

    #[tokio::test]
    async fn destruct_snapshot_returns_none_for_unknown_chat() {
        let state = make_state();
        let snapshot = state.destruct_snapshot("999").await;
        assert!(snapshot.is_none());
    }

    #[tokio::test]
    async fn security_file_start_sets_pending() {
        let state = make_state();
        state
            .start_security_file_input("42".into(), Instant::now())
            .await;
        assert_eq!(
            state
                .take_security_file_input_status("42", Duration::from_secs(60))
                .await,
            TimeoutStatus::Active
        );
    }

    #[tokio::test]
    async fn security_file_take_after_timeout_returns_expired() {
        let state = make_state();
        let past = Instant::now() - Duration::from_secs(120);
        state.start_security_file_input("42".into(), past).await;
        assert_eq!(
            state
                .take_security_file_input_status("42", Duration::from_secs(60))
                .await,
            TimeoutStatus::Expired
        );
    }

    #[tokio::test]
    async fn security_file_take_when_not_started_returns_not_tracked() {
        let state = make_state();
        assert_eq!(
            state
                .take_security_file_input_status("99", Duration::from_secs(60))
                .await,
            TimeoutStatus::NotTracked
        );
    }

    #[tokio::test]
    async fn domain_input_tracks_source_and_timeout() {
        let state = make_state();
        state
            .start_domain_input("chat".into(), DomainFlowSource::OneClick, Instant::now())
            .await;
        let input = state.domain_input_snapshot("chat").await.unwrap();
        assert_eq!(input.source, DomainFlowSource::OneClick);
        assert_eq!(input.step, DomainInputStep::AwaitDomain);
        assert_eq!(
            state
                .domain_timeout_status("chat", Duration::from_secs(120))
                .await,
            TimeoutStatus::Active
        );
    }

    #[tokio::test]
    async fn take_domain_input_removes_flow() {
        let state = make_state();
        state
            .start_domain_input("chat".into(), DomainFlowSource::Standalone, Instant::now())
            .await;
        assert!(state.take_domain_input("chat").await.is_some());
        assert!(state.domain_input_snapshot("chat").await.is_none());
    }

    #[tokio::test]
    async fn discord_admin_id_is_recognized_as_admin() {
        let state = AppState::new(
            Some(42),
            Some(999),
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        );
        assert!(state.is_admin_user(999));
        assert!(!state.is_admin_user(888));
    }
}
