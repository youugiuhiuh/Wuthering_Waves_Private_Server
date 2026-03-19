use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use teloxide::types::ChatId;
use tokio::sync::Mutex;

use crate::logic::scheduler::task_types::TaskType;
use crate::logic::self_destruct::SelfDestructExecutor;
use crate::logic::totp::TotpManager;

const RECENT_AUTH_WINDOW_SECS: u64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructStep {
    AwaitFirstTotp,
    AwaitConfirm,
    AwaitSecondTotp,
    AwaitSecurityFile,
    AwaitFinalConfirm,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStatus {
    NotTracked,
    Active,
    Expired,
}

#[derive(Debug, Clone)]
pub struct DestructState {
    pub step: DestructStep,
    pub first_totp: String,
    pub second_totp: String,
    pub last_action_time: Instant,
}

#[derive(Debug, Clone)]
pub struct FailedRecord {
    pub count: u32,
    pub first_fail: Instant,
    pub cooldown_until: Option<Instant>,
    pub lock_level: usize,
}

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
    admin_id: i64,
    totp_manager: TotpManager,
    self_destruct_executor: Arc<dyn SelfDestructExecutor>,
    sessions: Mutex<HashMap<i64, Instant>>,
    failed_attempts: Mutex<HashMap<i64, FailedRecord>>,
    pending_destructs: Mutex<HashMap<ChatId, DestructState>>,
    self_destruct_key_hash: Mutex<Option<String>>,
    pending_warp_inputs: Mutex<HashMap<ChatId, Instant>>,
    pending_schedule_inputs: Mutex<HashMap<ChatId, ScheduleInputState>>,
    session_timeout_secs: Mutex<u64>,
}

impl AppState {
    pub fn new(
        admin_id: i64,
        totp_manager: TotpManager,
        self_destruct_executor: Arc<dyn SelfDestructExecutor>,
        self_destruct_key_hash: Option<String>,
        session_timeout_secs: u64,
    ) -> Self {
        Self {
            admin_id,
            totp_manager,
            self_destruct_executor,
            sessions: Mutex::new(HashMap::new()),
            failed_attempts: Mutex::new(HashMap::new()),
            pending_destructs: Mutex::new(HashMap::new()),
            self_destruct_key_hash: Mutex::new(self_destruct_key_hash),
            pending_warp_inputs: Mutex::new(HashMap::new()),
            pending_schedule_inputs: Mutex::new(HashMap::new()),
            session_timeout_secs: Mutex::new(session_timeout_secs),
        }
    }

    pub fn admin_id(&self) -> i64 {
        self.admin_id
    }

    pub fn is_admin_user(&self, user_id: i64) -> bool {
        user_id == self.admin_id
    }

    pub fn verify_totp(&self, code: &str) -> bool {
        self.totp_manager.verify(code)
    }

    pub async fn is_authorized(&self, user_id: i64) -> bool {
        if !self.is_admin_user(user_id) {
            return false;
        }

        let timeout = self.session_timeout_secs().await;
        let sessions = self.sessions.lock().await;
        sessions
            .get(&user_id)
            .map(|t| t.elapsed() < Duration::from_secs(timeout))
            .unwrap_or(false)
    }

    pub async fn is_recently_authenticated(&self, user_id: i64) -> bool {
        if !self.is_admin_user(user_id) {
            return false;
        }

        let sessions = self.sessions.lock().await;
        sessions
            .get(&user_id)
            .map(|t| t.elapsed() < Duration::from_secs(RECENT_AUTH_WINDOW_SECS))
            .unwrap_or(false)
    }

    pub async fn session_timeout_secs(&self) -> u64 {
        *self.session_timeout_secs.lock().await
    }

    pub async fn set_session_timeout_secs(&self, secs: u64) {
        *self.session_timeout_secs.lock().await = secs;
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

    pub async fn begin_destruct(&self, chat_id: ChatId, now: Instant) {
        self.pending_destructs.lock().await.insert(
            chat_id,
            DestructState {
                step: DestructStep::AwaitFirstTotp,
                first_totp: String::new(),
                second_totp: String::new(),
                last_action_time: now,
            },
        );
    }

    pub async fn cancel_destruct(&self, chat_id: ChatId) -> bool {
        self.pending_destructs
            .lock()
            .await
            .remove(&chat_id)
            .is_some()
    }

    pub async fn destruct_snapshot(&self, chat_id: ChatId) -> Option<DestructState> {
        self.pending_destructs.lock().await.get(&chat_id).cloned()
    }

    pub async fn touch_destruct(
        &self,
        chat_id: ChatId,
        now: Instant,
        timeout: Duration,
    ) -> TimeoutStatus {
        let mut destructs = self.pending_destructs.lock().await;
        match destructs.get_mut(&chat_id) {
            Some(state) if state.last_action_time.elapsed() > timeout => TimeoutStatus::Expired,
            Some(state) => {
                state.last_action_time = now;
                TimeoutStatus::Active
            }
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn advance_destruct_step(
        &self,
        chat_id: ChatId,
        expected: DestructStep,
        next: DestructStep,
        now: Instant,
    ) -> bool {
        self.with_destruct(chat_id, |state| {
            if state.step == expected {
                state.step = next;
                state.last_action_time = now;
                true
            } else {
                false
            }
        })
        .await
        .unwrap_or(false)
    }

    pub async fn confirm_first_destruct_totp(
        &self,
        chat_id: ChatId,
        code: &str,
        now: Instant,
    ) -> bool {
        self.with_destruct(chat_id, |state| {
            if state.step == DestructStep::AwaitFirstTotp {
                state.step = DestructStep::AwaitConfirm;
                state.first_totp = code.to_string();
                state.last_action_time = now;
                true
            } else {
                false
            }
        })
        .await
        .unwrap_or(false)
    }

    pub async fn confirm_second_destruct_totp(
        &self,
        chat_id: ChatId,
        code: &str,
        now: Instant,
    ) -> Result<bool, String> {
        let snapshot = self.destruct_snapshot(chat_id).await;
        let Some(snapshot) = snapshot else {
            return Ok(false);
        };
        if snapshot.step != DestructStep::AwaitSecondTotp {
            return Ok(false);
        }
        if snapshot.first_totp == code {
            return Err(snapshot.first_totp);
        }

        Ok(self
            .with_destruct(chat_id, |state| {
                if state.step == DestructStep::AwaitSecondTotp {
                    state.step = DestructStep::AwaitSecurityFile;
                    state.second_totp = code.to_string();
                    state.last_action_time = now;
                    true
                } else {
                    false
                }
            })
            .await
            .unwrap_or(false))
    }

    pub async fn mark_destruct_file_verified(&self, chat_id: ChatId, now: Instant) -> bool {
        self.advance_destruct_step(
            chat_id,
            DestructStep::AwaitSecurityFile,
            DestructStep::AwaitFinalConfirm,
            now,
        )
        .await
    }

    pub async fn with_destruct<R>(
        &self,
        chat_id: ChatId,
        f: impl FnOnce(&mut DestructState) -> R,
    ) -> Option<R> {
        let mut destructs = self.pending_destructs.lock().await;
        destructs.get_mut(&chat_id).map(f)
    }

    pub async fn start_warp_input(&self, chat_id: ChatId, now: Instant) {
        self.pending_warp_inputs.lock().await.insert(chat_id, now);
    }

    pub async fn take_warp_input_status(
        &self,
        chat_id: ChatId,
        timeout: Duration,
    ) -> TimeoutStatus {
        let mut warp_inputs = self.pending_warp_inputs.lock().await;
        match warp_inputs.remove(&chat_id) {
            Some(start_time) if start_time.elapsed() > timeout => TimeoutStatus::Expired,
            Some(_) => TimeoutStatus::Active,
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn schedule_timeout_status(
        &self,
        chat_id: ChatId,
        timeout: Duration,
    ) -> TimeoutStatus {
        let schedule_inputs = self.pending_schedule_inputs.lock().await;
        match schedule_inputs.get(&chat_id) {
            Some(input) if input.updated_at.elapsed() > timeout => TimeoutStatus::Expired,
            Some(_) => TimeoutStatus::Active,
            None => TimeoutStatus::NotTracked,
        }
    }

    pub async fn remove_schedule_input(&self, chat_id: ChatId) {
        self.pending_schedule_inputs.lock().await.remove(&chat_id);
    }

    pub async fn insert_schedule_input(&self, chat_id: ChatId, input: ScheduleInputState) {
        self.pending_schedule_inputs
            .lock()
            .await
            .insert(chat_id, input);
    }

    pub async fn schedule_input_snapshot(&self, chat_id: ChatId) -> Option<ScheduleInputState> {
        self.pending_schedule_inputs
            .lock()
            .await
            .get(&chat_id)
            .cloned()
    }

    pub async fn with_schedule_input<R>(
        &self,
        chat_id: ChatId,
        f: impl FnOnce(&mut ScheduleInputState) -> R,
    ) -> Option<R> {
        let mut inputs = self.pending_schedule_inputs.lock().await;
        inputs.get_mut(&chat_id).map(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use futures_util::future::BoxFuture;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
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
        let chat_id = ChatId(1);
        let now = Instant::now();
        state.begin_destruct(chat_id, now).await;
        assert!(
            state
                .confirm_first_destruct_totp(chat_id, "111111", now)
                .await
        );
        assert!(
            state
                .advance_destruct_step(
                    chat_id,
                    DestructStep::AwaitConfirm,
                    DestructStep::AwaitSecondTotp,
                    now,
                )
                .await
        );
        assert!(
            state
                .confirm_second_destruct_totp(chat_id, "222222", now)
                .await
                .unwrap()
        );
        assert!(state.mark_destruct_file_verified(chat_id, now).await);
    }

    #[tokio::test]
    async fn destruct_timeout_is_detected() {
        let state = make_state();
        let chat_id = ChatId(2);
        state
            .begin_destruct(chat_id, Instant::now() - Duration::from_secs(61))
            .await;
        assert_eq!(
            state
                .touch_destruct(chat_id, Instant::now(), Duration::from_secs(60))
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
    async fn schedule_timeout_returns_not_tracked_for_unknown_chat() {
        let state = make_state();
        let status = state
            .schedule_timeout_status(ChatId(123), Duration::from_secs(60))
            .await;
        assert_eq!(status, TimeoutStatus::NotTracked);
    }

    #[tokio::test]
    async fn schedule_input_insert_and_snapshot() {
        let state = make_state();
        let chat_id = ChatId(100);
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
        state.insert_schedule_input(chat_id, input.clone()).await;
        let snapshot = state.schedule_input_snapshot(chat_id).await;
        assert!(snapshot.is_some());
        let snap = snapshot.unwrap();
        assert_eq!(snap.task_type, TaskType::Reboot);
        assert_eq!(snap.hour, Some(3));
    }

    #[tokio::test]
    async fn cancel_destruct_returns_true_when_exists() {
        let state = make_state();
        let chat_id = ChatId(5);
        state.begin_destruct(chat_id, Instant::now()).await;
        assert!(state.cancel_destruct(chat_id).await);
    }

    #[tokio::test]
    async fn cancel_destruct_returns_false_when_not_exists() {
        let state = make_state();
        assert!(!state.cancel_destruct(ChatId(999)).await);
    }

    #[tokio::test]
    async fn warp_input_timeout_tracking() {
        let state = make_state();
        let chat_id = ChatId(200);
        state.start_warp_input(chat_id, Instant::now()).await;
        let status = state
            .take_warp_input_status(chat_id, Duration::from_secs(60))
            .await;
        assert_eq!(status, TimeoutStatus::Active);
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
        let snapshot = state.destruct_snapshot(ChatId(999)).await;
        assert!(snapshot.is_none());
    }
}
