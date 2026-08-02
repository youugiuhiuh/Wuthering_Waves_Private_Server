//! Schedule-creation state machine, keyed by conversation identity.
//!
//! Ownership moves here from `AppState` so the workflow can be tested
//! independently of any gateway. Transitions: start, input, expiry, completion,
//! and cancellation. Scheduling execution stays in the core scheduler service.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::app::interaction::ConversationId;
use crate::core::system::scheduler::task_types::TaskType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleFrequency {
    Daily,
    Weekly,
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

/// Semantic outcome of routing a plain message through the schedule workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleFlow {
    /// Nothing tracked for this conversation; message dispatch continues.
    Continue,
    /// A schedule input is pending; the handler consumes the message and may
    /// show the input prompt.
    Waiting,
    /// The pending input expired; the handler notifies and the flow is closed.
    Expired,
}

/// Schedule-creation state machine keyed by conversation identity.
/// Transitions: start, input, expiry, completion, cancellation.
#[derive(Default)]
pub struct ScheduleWorkflow {
    pending: Mutex<HashMap<ConversationId, ScheduleInputState>>,
}

impl ScheduleWorkflow {
    pub fn new() -> Self {
        Self::default()
    }

    /// start: begin collecting schedule input for a conversation.
    pub fn start(&self, conversation: ConversationId, input: ScheduleInputState) {
        self.pending.lock().unwrap().insert(conversation, input);
    }

    /// input: record a collected field, returning the mutation result when the
    /// conversation has an active flow.
    pub fn input<R>(
        &self,
        conversation: &ConversationId,
        f: impl FnOnce(&mut ScheduleInputState) -> R,
    ) -> Option<R> {
        self.pending.lock().unwrap().get_mut(conversation).map(f)
    }

    /// Snapshot of the pending input, if any.
    pub fn snapshot(&self, conversation: &ConversationId) -> Option<ScheduleInputState> {
        self.pending.lock().unwrap().get(conversation).cloned()
    }

    /// expiry: route an inbound message. Expired flows are removed here so the
    /// handler only turns the semantic outcome into a reply.
    pub fn route(&self, conversation: &ConversationId, timeout: Duration) -> ScheduleFlow {
        let mut pending = self.pending.lock().unwrap();
        match pending.get(conversation) {
            Some(input) if input.updated_at.elapsed() > timeout => {
                pending.remove(conversation);
                ScheduleFlow::Expired
            }
            Some(_) => ScheduleFlow::Waiting,
            None => ScheduleFlow::Continue,
        }
    }

    /// completion: finish collection for a conversation.
    pub fn complete(&self, conversation: &ConversationId) -> bool {
        self.pending.lock().unwrap().remove(conversation).is_some()
    }

    /// cancellation: abort collection for a conversation.
    pub fn cancel(&self, conversation: &ConversationId) -> bool {
        self.pending.lock().unwrap().remove(conversation).is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::app::interaction::ConversationId;

    fn conversation(id: &str) -> ConversationId {
        ConversationId::new(id.to_string()).unwrap()
    }

    fn input(task_type: TaskType) -> ScheduleInputState {
        ScheduleInputState {
            updated_at: Instant::now(),
            task_type,
            frequency: ScheduleFrequency::Daily,
            timezone: "UTC".to_string(),
            day_of_week: None,
            hour: Some(3),
            minute: Some(0),
            return_to: "m_main".to_string(),
        }
    }

    #[test]
    fn transitions_follow_expected_ordering() {
        let wf = ScheduleWorkflow::new();
        let conv = conversation("100");

        wf.start(conv.clone(), input(TaskType::Reboot));
        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Waiting
        );
        assert!(wf.snapshot(&conv).is_some());

        let hour = wf.input(&conv, |s| {
            s.hour = Some(5);
            s.hour
        });
        assert_eq!(hour, Some(Some(5)));
        assert_eq!(wf.snapshot(&conv).unwrap().hour, Some(5));

        assert!(wf.complete(&conv));
        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Continue
        );
        assert!(wf.snapshot(&conv).is_none());
    }

    #[test]
    fn invalid_ordering_is_rejected() {
        let wf = ScheduleWorkflow::new();
        let conv = conversation("101");

        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Continue
        );
        assert!(wf.input(&conv, |s| s.hour = Some(1)).is_none());
        assert!(!wf.complete(&conv));
        assert!(!wf.cancel(&conv));
    }

    #[test]
    fn timeout_expires_and_clears() {
        let wf = ScheduleWorkflow::new();
        let conv = conversation("102");
        let mut stale = input(TaskType::Reboot);
        stale.updated_at = Instant::now() - Duration::from_secs(181);
        wf.start(conv.clone(), stale);

        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Expired
        );
        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Continue
        );
        assert!(wf.snapshot(&conv).is_none());
    }

    #[test]
    fn cancel_removes_active_flow() {
        let wf = ScheduleWorkflow::new();
        let conv = conversation("103");
        wf.start(conv.clone(), input(TaskType::Reboot));
        assert!(wf.cancel(&conv));
        assert_eq!(
            wf.route(&conv, Duration::from_secs(180)),
            ScheduleFlow::Continue
        );
    }

    #[test]
    fn expiry_decision_is_table_driven() {
        let cases = [
            (Duration::from_secs(0), ScheduleFlow::Expired),
            (Duration::from_secs(3600), ScheduleFlow::Waiting),
        ];
        for (timeout, expected) in cases {
            let wf = ScheduleWorkflow::new();
            let conv = conversation("104");
            let mut stale = input(TaskType::Reboot);
            stale.updated_at = Instant::now() - Duration::from_secs(60);
            wf.start(conv.clone(), stale);
            assert_eq!(wf.route(&conv, timeout), expected);
        }
    }
}
