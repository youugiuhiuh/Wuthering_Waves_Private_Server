use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use aegis::adapters::common::{BotAdapter, DestructKey, Platform, Principal};
use aegis::core::i18n::Lang;
use aegis::core::security::self_destruct::SelfDestructExecutor;
use aegis::core::system::scheduler::task_types::TaskType;
use aegis::core::totp::TotpManager;
use aegis::shared::handlers::message::MessageState;
use aegis::shared::types::TimeoutStatus;

const RECENT_AUTH_WINDOW_SECS: u64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructStatus {
    AwaitFirstTotp,
    AwaitFirstConfirm,
    AwaitSecondTotp,
    AwaitSecurityFile,
    AwaitFinalConfirm,
    Cancelled,
    Expired,
    Locked,
    Executing,
    Succeeded,
    Failed,
}

impl DestructStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            DestructStatus::AwaitFirstTotp
                | DestructStatus::AwaitFirstConfirm
                | DestructStatus::AwaitSecondTotp
                | DestructStatus::AwaitSecurityFile
                | DestructStatus::AwaitFinalConfirm
        )
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleFrequency {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailureOutcome {
    Invalid { attempts: u32, max_attempts: u32 },
    Locked { duration: Duration },
}

#[derive(Debug, Clone)]
pub struct DestructState {
    pub status: DestructStatus,
    pub deadline: Instant,
    pub failed_attempts: u8,
    pub accepted_counters: Vec<u64>,
    pub final_nonce: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructFailure {
    Delay(Duration),
    Locked,
}

#[derive(Debug, Clone)]
pub struct FailedRecord {
    pub count: u32,
    pub first_fail: Instant,
    pub cooldown_until: Option<Instant>,
    pub lock_level: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ScheduleInputState {
    pub updated_at: Instant,
    pub task_type: TaskType,
    pub frequency: ScheduleFrequency,
    pub timezone: String,
    pub day_of_week: Option<String>,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub return_to: String,
}

pub struct AppState {
    #[allow(dead_code)]
    pub adapter: Arc<dyn BotAdapter>,
    admin_id: i64,
    discord_admin_id: Option<i64>,
    totp_manager: TotpManager,
    self_destruct_executor: Arc<dyn SelfDestructExecutor>,
    sessions: Mutex<HashMap<String, Instant>>,
    failed_attempts: Mutex<HashMap<String, FailedRecord>>,
    pending_destructs: Mutex<HashMap<DestructKey, DestructState>>,
    self_destruct_key_hash: Mutex<Option<String>>,
    pending_warp_inputs: Mutex<HashMap<String, Instant>>,
    pending_schedule_inputs: Mutex<HashMap<String, ScheduleInputState>>,
    pending_security_file: Mutex<HashMap<String, Instant>>,
    consumed_counters: Mutex<HashSet<u64>>,
    session_timeout_secs: Mutex<u64>,
    lang: Mutex<Lang>,
    lang_configured: Mutex<bool>,
}

impl AppState {
    pub fn new(
        admin_id: i64,
        discord_admin_id: Option<i64>,
        totp_manager: TotpManager,
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
            pending_destructs: Mutex::new(HashMap::new()),
            self_destruct_key_hash: Mutex::new(self_destruct_key_hash),
            pending_warp_inputs: Mutex::new(HashMap::new()),
            pending_schedule_inputs: Mutex::new(HashMap::new()),
            pending_security_file: Mutex::new(HashMap::new()),
            consumed_counters: Mutex::new(HashSet::new()),
            session_timeout_secs: Mutex::new(session_timeout_secs),
            lang: Mutex::new(Lang::Zh),
            lang_configured: Mutex::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn admin_id(&self) -> i64 {
        self.admin_id
    }

    pub fn is_admin_user(&self, principal: &Principal) -> bool {
        let uid: i64 = match principal.platform {
            Platform::Matrix => principal
                .subject
                .trim_start_matches('@')
                .split(':')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0),
            _ => principal.subject.parse().unwrap_or(0),
        };
        uid == self.admin_id || self.discord_admin_id == Some(uid)
    }

    pub fn verify_totp(&self, code: &str) -> bool {
        self.totp_manager.verify(code)
    }

    #[allow(dead_code)]
    pub fn generate_current_totp(&self) -> Result<String, std::time::SystemTimeError> {
        self.totp_manager.generate_current()
    }

    pub async fn is_authorized(&self, principal: &Principal) -> bool {
        if !self.is_admin_user(principal) {
            return false;
        }

        let timeout = self.session_timeout_secs().await;
        let sessions = self.sessions.lock().await;
        sessions
            .get(&principal.key())
            .map(|t| is_session_valid(t, timeout))
            .unwrap_or(false)
    }

    pub async fn is_recently_authenticated(&self, principal: &Principal) -> bool {
        if !self.is_admin_user(principal) {
            return false;
        }

        let sessions = self.sessions.lock().await;
        sessions
            .get(&principal.key())
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

    pub async fn record_auth_success(&self, principal: &Principal, now: Instant) -> u64 {
        let key = principal.key();
        self.sessions.lock().await.insert(key.clone(), now);
        self.failed_attempts.lock().await.remove(&key);
        self.session_timeout_secs().await
    }

    pub async fn record_auth_failure(
        &self,
        principal: &Principal,
        now: Instant,
        max_attempts: u32,
        failure_window: Duration,
        lockout_durations: &[Duration],
    ) -> AuthFailureOutcome {
        let key = principal.key();
        let mut fails = self.failed_attempts.lock().await;
        let rec = fails.entry(key).or_insert(FailedRecord {
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

    pub async fn auth_cooldown_remaining(
        &self,
        principal: &Principal,
        now: Instant,
    ) -> Option<Duration> {
        let key = principal.key();
        let mut fails = self.failed_attempts.lock().await;
        let rec = fails.get_mut(&key)?;
        let until = rec.cooldown_until?;
        if until > now {
            Some(until - now)
        } else {
            rec.cooldown_until = None;
            None
        }
    }

    pub async fn begin_destruct(&self, key: &DestructKey, now: Instant) -> Result<(), String> {
        let hash = self.self_destruct_key_hash.lock().await;
        if hash.is_none() {
            return Err("self-destruct key not configured".to_string());
        }
        drop(hash);

        self.pending_destructs.lock().await.insert(
            key.clone(),
            DestructState {
                status: DestructStatus::AwaitFirstTotp,
                deadline: now + Duration::from_secs(300),
                failed_attempts: 0,
                accepted_counters: Vec::new(),
                final_nonce: None,
            },
        );
        Ok(())
    }

    pub async fn cancel_destruct(&self, key: &DestructKey) -> bool {
        self.pending_destructs.lock().await.remove(key).is_some()
    }

    pub async fn destruct_snapshot(&self, key: &DestructKey) -> Option<DestructState> {
        self.pending_destructs.lock().await.get(key).cloned()
    }

    pub async fn ensure_destruct_active(
        &self,
        key: &DestructKey,
        now: Instant,
    ) -> Result<(), String> {
        let destructs = self.pending_destructs.lock().await;
        let state = destructs
            .get(key)
            .ok_or_else(|| "no pending destruct".to_string())?;
        if !state.status.is_active() {
            return Err("destruct not in active state".to_string());
        }
        if state.deadline <= now {
            return Err("destruct deadline expired".to_string());
        }
        Ok(())
    }

    pub async fn record_destruct_failure(&self, key: &DestructKey) -> Result<(), DestructFailure> {
        let mut destructs = self.pending_destructs.lock().await;
        let state = destructs.get_mut(key).ok_or(DestructFailure::Locked)?;

        match state.failed_attempts {
            0..=2 => {
                state.failed_attempts += 1;
                Err(DestructFailure::Delay(Duration::from_secs(
                    1 << (state.failed_attempts - 1),
                )))
            }
            _ => {
                state.status = DestructStatus::Locked;
                Err(DestructFailure::Locked)
            }
        }
    }

    pub async fn advance_destruct_step(
        &self,
        key: &DestructKey,
        expected: DestructStatus,
        next: DestructStatus,
    ) -> bool {
        self.with_destruct(key, |state| {
            if state.status == expected {
                state.status = next;
                true
            } else {
                false
            }
        })
        .await
        .unwrap_or(false)
    }

    pub async fn with_destruct<R>(
        &self,
        key: &DestructKey,
        f: impl FnOnce(&mut DestructState) -> R,
    ) -> Option<R> {
        let mut destructs = self.pending_destructs.lock().await;
        destructs.get_mut(key).map(f)
    }

    pub async fn start_warp_input(&self, chat_id: String, now: Instant) {
        self.pending_warp_inputs.lock().await.insert(chat_id, now);
    }

    pub async fn take_warp_input_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        let mut warp_inputs = self.pending_warp_inputs.lock().await;
        match warp_inputs.remove(chat_id) {
            Some(start_time) if start_time.elapsed() > timeout => TimeoutStatus::Expired,
            Some(_) => TimeoutStatus::Active,
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn schedule_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        let schedule_inputs = self.pending_schedule_inputs.lock().await;
        match schedule_inputs.get(chat_id) {
            Some(input) if input.updated_at.elapsed() > timeout => TimeoutStatus::Expired,
            Some(_) => TimeoutStatus::Active,
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn remove_schedule_input(&self, chat_id: &str) {
        self.pending_schedule_inputs.lock().await.remove(chat_id);
    }

    #[allow(dead_code)]
    pub async fn insert_schedule_input(&self, chat_id: String, input: ScheduleInputState) {
        self.pending_schedule_inputs
            .lock()
            .await
            .insert(chat_id, input);
    }

    #[allow(dead_code)]
    pub async fn schedule_input_snapshot(&self, chat_id: &str) -> Option<ScheduleInputState> {
        self.pending_schedule_inputs
            .lock()
            .await
            .get(chat_id)
            .cloned()
    }

    #[allow(dead_code)]
    pub async fn with_schedule_input<R>(
        &self,
        chat_id: &str,
        f: impl FnOnce(&mut ScheduleInputState) -> R,
    ) -> Option<R> {
        let mut inputs = self.pending_schedule_inputs.lock().await;
        inputs.get_mut(chat_id).map(f)
    }

    pub async fn consume_totp_counter(
        &self,
        counter: u64,
        now: u64,
        skew_window: u64,
    ) -> anyhow::Result<()> {
        let mut counters = self.consumed_counters.lock().await;
        let threshold = (now / 30).saturating_sub(skew_window);
        counters.retain(|&c| c >= threshold);
        if !counters.insert(counter) {
            anyhow::bail!("counter {} already consumed", counter);
        }
        Ok(())
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
}

#[async_trait]
impl MessageState for AppState {
    async fn schedule_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        self.schedule_timeout_status(chat_id, timeout).await
    }

    async fn remove_schedule_input(&self, chat_id: &str) {
        self.remove_schedule_input(chat_id).await
    }

    async fn take_warp_input_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus {
        self.take_warp_input_status(chat_id, timeout).await
    }
}

fn is_session_valid(session_time: &Instant, timeout_secs: u64) -> bool {
    session_time.elapsed() < Duration::from_secs(timeout_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::adapters::common::{MessageContent, MessageId, Platform, TargetId};
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
        fn capabilities(&self) -> aegis::adapters::common::PlatformCapabilities {
            aegis::adapters::common::PlatformCapabilities::TELEGRAM
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            42,
            None,
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        )
    }

    fn make_state_with_key_hash() -> AppState {
        AppState::new(
            42,
            None,
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            Some("test-hash".to_string()),
            600,
            Arc::new(MockAdapter),
        )
    }

    fn p(uid: i64) -> Principal {
        Principal::telegram(uid as u64)
    }

    #[tokio::test]
    async fn authorization_success_creates_valid_session() {
        let state = make_state();
        state.record_auth_success(&p(42), Instant::now()).await;
        assert!(state.is_authorized(&p(42)).await);
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
                    .record_auth_failure(&p(42), now, 5, Duration::from_secs(600), &lockouts)
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
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("1".into()),
        };
        let now = Instant::now();
        state.begin_destruct(&key, now).await.unwrap();
        assert!(
            state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitFirstTotp,
                    DestructStatus::AwaitFirstConfirm,
                )
                .await
        );
        assert!(
            state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitFirstConfirm,
                    DestructStatus::AwaitSecondTotp,
                )
                .await
        );
        assert!(
            state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitSecondTotp,
                    DestructStatus::AwaitSecurityFile,
                )
                .await
        );
        assert!(
            state
                .advance_destruct_step(
                    &key,
                    DestructStatus::AwaitSecurityFile,
                    DestructStatus::AwaitFinalConfirm,
                )
                .await
        );
    }

    #[tokio::test]
    async fn destruct_timeout_is_detected() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("2".into()),
        };
        let past = Instant::now() - Duration::from_secs(301);
        state.begin_destruct(&key, past).await.unwrap();
        assert!(
            state
                .ensure_destruct_active(&key, Instant::now())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn is_authorized_returns_false_for_non_admin() {
        let state = make_state();
        assert!(!state.is_authorized(&p(999)).await);
    }

    #[tokio::test]
    async fn is_authorized_returns_false_for_expired_session() {
        let state = make_state();
        let past = Instant::now() - Duration::from_secs(601);
        state.record_auth_success(&p(42), past).await;
        assert!(!state.is_authorized(&p(42)).await);
    }

    #[tokio::test]
    async fn auth_cooldown_remaining_returns_none_when_not_locked() {
        let state = make_state();
        let remaining = state.auth_cooldown_remaining(&p(42), Instant::now()).await;
        assert!(remaining.is_none());
    }

    #[tokio::test]
    async fn auth_cooldown_remaining_returns_duration_when_locked() {
        let state = make_state();
        let now = Instant::now();
        let lockouts = [Duration::from_secs(60)];
        for _ in 0..5 {
            state
                .record_auth_failure(&p(42), now, 5, Duration::from_secs(600), &lockouts)
                .await;
        }
        let remaining = state.auth_cooldown_remaining(&p(42), now).await;
        assert!(remaining.is_some());
        assert!(remaining.unwrap().as_secs() <= 60);
    }

    #[tokio::test]
    async fn schedule_timeout_returns_not_tracked_for_unknown_chat() {
        let state = make_state();
        let status = state
            .schedule_timeout_status("123", Duration::from_secs(60))
            .await;
        assert_eq!(status, TimeoutStatus::NotTracked);
    }

    #[tokio::test]
    async fn schedule_input_insert_and_snapshot() {
        let state = make_state();
        let chat_id = "100".to_string();
        let input = ScheduleInputState {
            updated_at: Instant::now(),
            task_type: TaskType::Reboot,
            frequency: ScheduleFrequency::Daily,
            timezone: "UTC".to_string(),
            day_of_week: None,
            hour: Some(3),
            minute: Some(0),
            return_to: "m_main".to_string(),
        };
        state
            .insert_schedule_input(chat_id.clone(), input.clone())
            .await;
        let snapshot = state.schedule_input_snapshot(&chat_id).await;
        assert!(snapshot.is_some());
        let snap = snapshot.unwrap();
        assert_eq!(snap.task_type, TaskType::Reboot);
        assert_eq!(snap.hour, Some(3));
    }

    #[tokio::test]
    async fn cancel_destruct_returns_true_when_exists() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("5".into()),
        };
        state.begin_destruct(&key, Instant::now()).await.unwrap();
        assert!(state.cancel_destruct(&key).await);
    }

    #[tokio::test]
    async fn cancel_destruct_returns_false_when_not_exists() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(999),
            target: TargetId("999".into()),
        };
        assert!(!state.cancel_destruct(&key).await);
    }

    #[tokio::test]
    async fn warp_input_timeout_tracking() {
        let state = make_state();
        let chat_id = "200".to_string();
        state
            .start_warp_input(chat_id.clone(), Instant::now())
            .await;
        let status = state
            .take_warp_input_status(&chat_id, Duration::from_secs(60))
            .await;
        assert_eq!(status, TimeoutStatus::Active);
    }

    #[tokio::test]
    async fn auth_failure_window_resets_count() {
        let state = make_state();
        let now = Instant::now();
        let lockouts = [Duration::from_secs(60)];
        state
            .record_auth_failure(&p(42), now, 5, Duration::from_secs(600), &lockouts)
            .await;
        state
            .record_auth_failure(&p(42), now, 5, Duration::from_secs(600), &lockouts)
            .await;
        let far_future = now + Duration::from_secs(601);
        let outcome = state
            .record_auth_failure(&p(42), far_future, 5, Duration::from_secs(600), &lockouts)
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
    async fn destruct_snapshot_returns_none_for_unknown_key() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(999),
            target: TargetId("999".into()),
        };
        let snapshot = state.destruct_snapshot(&key).await;
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
    async fn consume_totp_counter_accepts_new_counter() {
        let state = make_state();
        let now = 1_800_000_000u64;
        let counter = now / 30;
        assert!(state.consume_totp_counter(counter, now, 1).await.is_ok());
    }

    #[tokio::test]
    async fn consume_totp_counter_rejects_duplicate() {
        let state = make_state();
        let now = 1_800_000_000u64;
        let counter = now / 30;
        state.consume_totp_counter(counter, now, 1).await.unwrap();
        assert!(state.consume_totp_counter(counter, now, 1).await.is_err());
    }

    #[tokio::test]
    async fn consume_totp_counter_prunes_old_counters() {
        let state = make_state();
        let now = 1_800_000_000u64;
        let old_counter = now / 30 - 10;
        let current_counter = now / 30;

        state
            .consume_totp_counter(old_counter, now, 1)
            .await
            .unwrap();
        state
            .consume_totp_counter(current_counter, now, 1)
            .await
            .unwrap();
        assert!(
            state
                .consume_totp_counter(old_counter, now, 1)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn discord_admin_id_is_recognized_as_admin() {
        let state = AppState::new(
            42,
            Some(999),
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockAdapter),
        );
        assert!(state.is_admin_user(&Principal::discord(999)));
        assert!(!state.is_admin_user(&Principal::discord(888)));
    }

    #[tokio::test]
    async fn destruct_key_identity_distinguishes_principal_and_target() {
        let key1 = DestructKey {
            principal: p(1),
            target: TargetId("a".into()),
        };
        let key2 = DestructKey {
            principal: p(2),
            target: TargetId("a".into()),
        };
        let key3 = DestructKey {
            principal: p(1),
            target: TargetId("b".into()),
        };
        let key4 = DestructKey {
            principal: p(1),
            target: TargetId("a".into()),
        };
        assert_ne!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1, key4);
    }

    #[tokio::test]
    async fn begin_destruct_sets_300s_deadline() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("deadline1".into()),
        };
        let now = Instant::now();
        state.begin_destruct(&key, now).await.unwrap();
        let snap = state.destruct_snapshot(&key).await.unwrap();
        assert_eq!(snap.deadline, now + Duration::from_secs(300));
    }

    #[tokio::test]
    async fn no_destruct_transition_changes_deadline() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("deadline2".into()),
        };
        let now = Instant::now();
        state.begin_destruct(&key, now).await.unwrap();
        let deadline = state.destruct_snapshot(&key).await.unwrap().deadline;
        state
            .advance_destruct_step(
                &key,
                DestructStatus::AwaitFirstTotp,
                DestructStatus::AwaitFirstConfirm,
            )
            .await;
        assert_eq!(
            state.destruct_snapshot(&key).await.unwrap().deadline,
            deadline
        );
        state
            .advance_destruct_step(
                &key,
                DestructStatus::AwaitFirstConfirm,
                DestructStatus::AwaitSecondTotp,
            )
            .await;
        assert_eq!(
            state.destruct_snapshot(&key).await.unwrap().deadline,
            deadline
        );
    }

    #[tokio::test]
    async fn begin_destruct_rejects_when_no_key_hash() {
        let state = make_state();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("nokey".into()),
        };
        assert!(state.begin_destruct(&key, Instant::now()).await.is_err());
    }

    #[tokio::test]
    async fn destruct_failure_delays_escalate() {
        let state = make_state_with_key_hash();
        let key = DestructKey {
            principal: p(1),
            target: TargetId("fail".into()),
        };
        state.begin_destruct(&key, Instant::now()).await.unwrap();
        let r1 = state.record_destruct_failure(&key).await;
        assert!(r1.is_err());
        assert_eq!(
            r1.unwrap_err(),
            DestructFailure::Delay(Duration::from_secs(1))
        );
        let r2 = state.record_destruct_failure(&key).await;
        assert!(r2.is_err());
        assert_eq!(
            r2.unwrap_err(),
            DestructFailure::Delay(Duration::from_secs(2))
        );
        let r3 = state.record_destruct_failure(&key).await;
        assert!(r3.is_err());
        assert_eq!(
            r3.unwrap_err(),
            DestructFailure::Delay(Duration::from_secs(4))
        );
        let r4 = state.record_destruct_failure(&key).await;
        assert!(r4.is_err());
        assert_eq!(r4.unwrap_err(), DestructFailure::Locked);
        let snap = state.destruct_snapshot(&key).await.unwrap();
        assert_eq!(snap.status, DestructStatus::Locked);
    }
}
