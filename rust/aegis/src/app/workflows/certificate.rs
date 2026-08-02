use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::app::interaction::ConversationId;
use crate::core::types::{DomainFlowSource, DomainInputState, DomainInputStep};
use crate::shared::types::TimeoutStatus;

#[derive(Default)]
pub struct CertificateWorkflow {
    pending: Mutex<HashMap<ConversationId, DomainInputState>>,
}

impl CertificateWorkflow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, conversation: ConversationId, source: DomainFlowSource, now: Instant) {
        self.pending.lock().unwrap().insert(
            conversation,
            DomainInputState {
                updated_at: now,
                source,
                step: DomainInputStep::AwaitDomain,
                domain: None,
            },
        );
    }

    pub fn snapshot(&self, conversation: &ConversationId) -> Option<DomainInputState> {
        self.pending.lock().unwrap().get(conversation).cloned()
    }

    /// Compare-and-set transition: only succeeds when the current step matches
    /// `expected`; on success updates the step, refreshes the timestamp, and
    /// stores the domain if one is supplied.
    pub fn transition(
        &self,
        conversation: &ConversationId,
        expected: DomainInputStep,
        next: DomainInputStep,
        domain: Option<String>,
    ) -> bool {
        let mut pending = self.pending.lock().unwrap();
        match pending.get_mut(conversation) {
            Some(state) if state.step == expected => {
                state.step = next;
                state.updated_at = Instant::now();
                if domain.is_some() {
                    state.domain = domain;
                }
                true
            }
            _ => false,
        }
    }

    pub fn take(&self, conversation: &ConversationId) -> Option<DomainInputState> {
        self.pending.lock().unwrap().remove(conversation)
    }

    pub fn timeout_status(
        &self,
        conversation: &ConversationId,
        timeout: Duration,
    ) -> TimeoutStatus {
        match self.pending.lock().unwrap().get(conversation) {
            Some(state) if state.updated_at.elapsed() > timeout => TimeoutStatus::Expired,
            Some(_) => TimeoutStatus::Active,
            None => TimeoutStatus::NotTracked,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::app::interaction::ConversationId;
    use crate::app::workflows::certificate::CertificateWorkflow;
    use crate::core::types::{DomainFlowSource, DomainInputStep};
    use crate::shared::types::TimeoutStatus;

    fn conversation(id: u64) -> ConversationId {
        ConversationId::new(id.to_string()).unwrap()
    }

    #[test]
    fn start_initializes_await_domain() {
        let wf = CertificateWorkflow::new();
        wf.start(conversation(1), DomainFlowSource::OneClick, Instant::now());
        let state = wf.snapshot(&conversation(1)).expect("state present");
        assert_eq!(state.source, DomainFlowSource::OneClick);
        assert_eq!(state.step, DomainInputStep::AwaitDomain);
        assert!(state.domain.is_none());
    }

    #[test]
    fn transition_is_compare_and_set() {
        let wf = CertificateWorkflow::new();
        wf.start(
            conversation(1),
            DomainFlowSource::Standalone,
            Instant::now(),
        );
        assert!(wf.transition(
            &conversation(1),
            DomainInputStep::AwaitDomain,
            DomainInputStep::AwaitProvider,
            Some("example.com".into()),
        ));
        let state = wf.snapshot(&conversation(1)).expect("state present");
        assert_eq!(state.step, DomainInputStep::AwaitProvider);
        assert_eq!(state.domain.as_deref(), Some("example.com"));
    }

    #[test]
    fn transition_rejects_wrong_expected_step() {
        let wf = CertificateWorkflow::new();
        wf.start(
            conversation(1),
            DomainFlowSource::Standalone,
            Instant::now(),
        );
        assert!(!wf.transition(
            &conversation(1),
            DomainInputStep::AwaitProvider,
            DomainInputStep::Processing,
            Some("example.com".into()),
        ));
        let state = wf.snapshot(&conversation(1)).expect("state present");
        assert_eq!(state.step, DomainInputStep::AwaitDomain);
        assert!(state.domain.is_none());
    }

    #[test]
    fn unknown_conversation_has_no_snapshot() {
        let wf = CertificateWorkflow::new();
        assert!(wf.snapshot(&conversation(9)).is_none());
        assert_eq!(
            wf.timeout_status(&conversation(9), Duration::from_secs(120)),
            TimeoutStatus::NotTracked
        );
    }

    #[test]
    fn timeout_status_expires_stale_flow() {
        let wf = CertificateWorkflow::new();
        wf.start(
            conversation(1),
            DomainFlowSource::Standalone,
            Instant::now() - Duration::from_secs(121),
        );
        assert_eq!(
            wf.timeout_status(&conversation(1), Duration::from_secs(120)),
            TimeoutStatus::Expired
        );
    }

    #[test]
    fn timeout_status_active_for_fresh_flow() {
        let wf = CertificateWorkflow::new();
        wf.start(
            conversation(1),
            DomainFlowSource::Standalone,
            Instant::now(),
        );
        assert_eq!(
            wf.timeout_status(&conversation(1), Duration::from_secs(120)),
            TimeoutStatus::Active
        );
    }

    #[test]
    fn take_removes_flow() {
        let wf = CertificateWorkflow::new();
        wf.start(
            conversation(1),
            DomainFlowSource::Standalone,
            Instant::now(),
        );
        assert!(wf.take(&conversation(1)).is_some());
        assert!(wf.snapshot(&conversation(1)).is_none());
    }
}
