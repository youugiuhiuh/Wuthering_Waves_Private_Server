use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::app::interaction::ConversationId;
use crate::common::r#trait::Platform;
use crate::shared::types::TimeoutStatus;

/// Origin-keyed self-destruct state machine. The machine only records
/// transitions and expiry; invoking the executor is the orchestrator's job
/// and happens only after an authorized, confirmed decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum DestructStep {
    AwaitFirstTotp,
    AwaitConfirm,
    AwaitSecondTotp,
    AwaitSecurityFile,
    AwaitFinalConfirm,
}

#[derive(Debug, Clone)]
pub struct DestructState {
    pub step: DestructStep,
    pub first_totp: String,
    pub second_totp: String,
    pub last_action_time: Instant,
}

#[derive(Default)]
pub struct DestructWorkflow {
    pending: Mutex<HashMap<(Platform, ConversationId), DestructState>>,
}

impl DestructWorkflow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&self, platform: Platform, conversation: ConversationId, now: Instant) {
        self.pending.lock().unwrap().insert(
            (platform, conversation),
            DestructState {
                step: DestructStep::AwaitFirstTotp,
                first_totp: String::new(),
                second_totp: String::new(),
                last_action_time: now,
            },
        );
    }

    pub fn cancel(&self, platform: Platform, conversation: &ConversationId) -> bool {
        self.pending
            .lock()
            .unwrap()
            .remove(&(platform, conversation.clone()))
            .is_some()
    }

    pub fn snapshot(
        &self,
        platform: Platform,
        conversation: &ConversationId,
    ) -> Option<DestructState> {
        self.pending
            .lock()
            .unwrap()
            .get(&(platform, conversation.clone()))
            .cloned()
    }

    pub fn touch(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        now: Instant,
        timeout: Duration,
    ) -> TimeoutStatus {
        let mut pending = self.pending.lock().unwrap();
        let key = (platform, conversation.clone());
        match pending.get_mut(&key) {
            Some(state) if state.last_action_time.elapsed() > timeout => TimeoutStatus::Expired,
            Some(state) => {
                state.last_action_time = now;
                TimeoutStatus::Active
            }
            None => TimeoutStatus::NotTracked,
        }
    }

    pub fn advance_step(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        expected: DestructStep,
        next: DestructStep,
        now: Instant,
    ) -> bool {
        self.with_state(platform, conversation, |state| {
            if state.step == expected {
                state.step = next;
                state.last_action_time = now;
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
    }

    pub fn confirm_first_totp(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        code: &str,
        now: Instant,
    ) -> bool {
        self.with_state(platform, conversation, |state| {
            if state.step == DestructStep::AwaitFirstTotp {
                state.step = DestructStep::AwaitConfirm;
                state.first_totp = code.to_string();
                state.last_action_time = now;
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
    }

    pub fn confirm_second_totp(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        code: &str,
        now: Instant,
    ) -> Result<bool, String> {
        let Some(snapshot) = self.snapshot(platform, conversation) else {
            return Ok(false);
        };
        if snapshot.step != DestructStep::AwaitSecondTotp {
            return Ok(false);
        }
        if snapshot.first_totp == code {
            return Err(snapshot.first_totp);
        }

        Ok(self
            .with_state(platform, conversation, |state| {
                if state.step == DestructStep::AwaitSecondTotp {
                    state.step = DestructStep::AwaitSecurityFile;
                    state.second_totp = code.to_string();
                    state.last_action_time = now;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false))
    }

    pub fn mark_file_verified(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        now: Instant,
    ) -> bool {
        self.advance_step(
            platform,
            conversation,
            DestructStep::AwaitSecurityFile,
            DestructStep::AwaitFinalConfirm,
            now,
        )
    }

    pub fn with_state<R>(
        &self,
        platform: Platform,
        conversation: &ConversationId,
        f: impl FnOnce(&mut DestructState) -> R,
    ) -> Option<R> {
        self.pending
            .lock()
            .unwrap()
            .get_mut(&(platform, conversation.clone()))
            .map(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interaction::ConversationId;
    use crate::common::r#trait::Platform;
    use crate::shared::types::TimeoutStatus;
    use std::time::{Duration, Instant};

    fn conversation(id: &str) -> ConversationId {
        ConversationId::new(id.to_string()).unwrap()
    }

    #[test]
    fn begin_initializes_await_first_totp() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        let snap = wf.snapshot(platform, &conversation("42")).unwrap();
        assert_eq!(snap.step, DestructStep::AwaitFirstTotp);
        assert!(snap.first_totp.is_empty());
        assert!(snap.second_totp.is_empty());
    }

    #[test]
    fn confirm_first_totp_advances_to_await_confirm() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        assert!(wf.confirm_first_totp(platform, &conversation("42"), "111111", Instant::now()));
        assert_eq!(
            wf.snapshot(platform, &conversation("42")).unwrap().step,
            DestructStep::AwaitConfirm
        );
        assert_eq!(
            wf.snapshot(platform, &conversation("42"))
                .unwrap()
                .first_totp,
            "111111"
        );
    }

    #[test]
    fn confirm_first_totp_rejects_wrong_step() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        wf.advance_step(
            platform,
            &conversation("42"),
            DestructStep::AwaitFirstTotp,
            DestructStep::AwaitConfirm,
            Instant::now(),
        );
        assert!(!wf.confirm_first_totp(platform, &conversation("42"), "111111", Instant::now()));
    }

    #[test]
    fn confirm_second_totp_requires_await_second_step() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        let result =
            wf.confirm_second_totp(platform, &conversation("42"), "222222", Instant::now());
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn confirm_second_totp_rejects_reused_first_code() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        wf.confirm_first_totp(platform, &conversation("42"), "111111", Instant::now());
        wf.advance_step(
            platform,
            &conversation("42"),
            DestructStep::AwaitConfirm,
            DestructStep::AwaitSecondTotp,
            Instant::now(),
        );
        assert!(
            wf.confirm_second_totp(platform, &conversation("42"), "111111", Instant::now())
                .is_err()
        );
    }

    #[test]
    fn confirm_second_totp_advances_to_await_security_file() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        wf.confirm_first_totp(platform, &conversation("42"), "111111", Instant::now());
        wf.advance_step(
            platform,
            &conversation("42"),
            DestructStep::AwaitConfirm,
            DestructStep::AwaitSecondTotp,
            Instant::now(),
        );
        assert!(
            wf.confirm_second_totp(platform, &conversation("42"), "222222", Instant::now())
                .unwrap()
        );
        assert_eq!(
            wf.snapshot(platform, &conversation("42")).unwrap().step,
            DestructStep::AwaitSecurityFile
        );
    }

    #[test]
    fn advance_step_guards_expected_step() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        assert!(!wf.advance_step(
            platform,
            &conversation("42"),
            DestructStep::AwaitConfirm,
            DestructStep::AwaitFinalConfirm,
            Instant::now()
        ));
        assert_eq!(
            wf.snapshot(platform, &conversation("42")).unwrap().step,
            DestructStep::AwaitFirstTotp
        );
    }

    #[test]
    fn touch_detects_expiry() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(
            platform,
            conversation("42"),
            Instant::now() - Duration::from_secs(61),
        );
        assert_eq!(
            wf.touch(
                platform,
                &conversation("42"),
                Instant::now(),
                Duration::from_secs(60)
            ),
            TimeoutStatus::Expired
        );
    }

    #[test]
    fn touch_refreshes_active_flow() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        assert_eq!(
            wf.touch(
                platform,
                &conversation("42"),
                Instant::now(),
                Duration::from_secs(60)
            ),
            TimeoutStatus::Active
        );
        assert_eq!(
            wf.touch(
                platform,
                &conversation("42"),
                Instant::now(),
                Duration::from_secs(60)
            ),
            TimeoutStatus::Active
        );
    }

    #[test]
    fn touch_unknown_is_not_tracked() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        assert_eq!(
            wf.touch(
                platform,
                &conversation("99"),
                Instant::now(),
                Duration::from_secs(60)
            ),
            TimeoutStatus::NotTracked
        );
    }

    #[test]
    fn mark_file_verified_requires_await_security_file() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        assert!(!wf.mark_file_verified(platform, &conversation("42"), Instant::now()));
    }

    #[test]
    fn cancel_removes_active_flow() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        assert!(wf.cancel(platform, &conversation("42")));
        assert!(wf.snapshot(platform, &conversation("42")).is_none());
    }

    #[test]
    fn cancel_unknown_returns_false() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        assert!(!wf.cancel(platform, &conversation("99")));
    }

    #[test]
    fn executor_transition_reached_only_after_confirmed_flow() {
        let wf = DestructWorkflow::new();
        let platform = Platform::Telegram;
        wf.begin(platform, conversation("42"), Instant::now());
        wf.confirm_first_totp(platform, &conversation("42"), "111111", Instant::now());
        wf.advance_step(
            platform,
            &conversation("42"),
            DestructStep::AwaitConfirm,
            DestructStep::AwaitSecondTotp,
            Instant::now(),
        );
        wf.confirm_second_totp(platform, &conversation("42"), "222222", Instant::now())
            .unwrap();
        assert!(wf.mark_file_verified(platform, &conversation("42"), Instant::now()));
        assert_eq!(
            wf.snapshot(platform, &conversation("42")).unwrap().step,
            DestructStep::AwaitFinalConfirm
        );
    }

    #[test]
    fn different_platforms_are_separate_flows() {
        let wf = DestructWorkflow::new();
        wf.begin(Platform::Telegram, conversation("42"), Instant::now());
        wf.begin(Platform::Discord, conversation("42"), Instant::now());
        assert_eq!(
            wf.snapshot(Platform::Telegram, &conversation("42"))
                .unwrap()
                .step,
            DestructStep::AwaitFirstTotp
        );
        assert_eq!(
            wf.snapshot(Platform::Discord, &conversation("42"))
                .unwrap()
                .step,
            DestructStep::AwaitFirstTotp
        );
        assert!(wf.confirm_first_totp(
            Platform::Telegram,
            &conversation("42"),
            "111111",
            Instant::now()
        ));
        assert_eq!(
            wf.snapshot(Platform::Telegram, &conversation("42"))
                .unwrap()
                .step,
            DestructStep::AwaitConfirm
        );
        assert_eq!(
            wf.snapshot(Platform::Discord, &conversation("42"))
                .unwrap()
                .step,
            DestructStep::AwaitFirstTotp
        );
    }
}
