use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::app::interaction::ConversationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpFlow {
    Continue,
    Waiting,
    Expired,
}

#[derive(Default)]
pub struct WarpWorkflow {
    pending: Mutex<HashMap<ConversationId, Instant>>,
}

impl WarpWorkflow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, conversation: ConversationId, now: Instant) {
        self.pending.lock().unwrap().insert(conversation, now);
    }

    /// Destructive read: consumes the pending warp input, reporting whether it
    /// is still within its timeout window.
    pub fn take(&self, conversation: &ConversationId, timeout: Duration) -> WarpFlow {
        match self.pending.lock().unwrap().remove(conversation) {
            Some(start) if start.elapsed() > timeout => WarpFlow::Expired,
            Some(_) => WarpFlow::Waiting,
            None => WarpFlow::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::app::interaction::ConversationId;
    use crate::app::workflows::warp::WarpFlow;
    use crate::app::workflows::warp::WarpWorkflow;

    fn conversation(id: u64) -> ConversationId {
        ConversationId::new(id.to_string()).unwrap()
    }

    #[test]
    fn start_then_take_is_waiting() {
        let wf = WarpWorkflow::new();
        wf.start(conversation(1), Instant::now());
        assert_eq!(
            wf.take(&conversation(1), Duration::from_secs(60)),
            WarpFlow::Waiting
        );
    }

    #[test]
    fn stale_start_expires() {
        let wf = WarpWorkflow::new();
        wf.start(conversation(1), Instant::now() - Duration::from_secs(61));
        assert_eq!(
            wf.take(&conversation(1), Duration::from_secs(60)),
            WarpFlow::Expired
        );
    }

    #[test]
    fn unknown_is_continue() {
        let wf = WarpWorkflow::new();
        assert_eq!(
            wf.take(&conversation(9), Duration::from_secs(60)),
            WarpFlow::Continue
        );
    }

    #[test]
    fn take_is_destructive() {
        let wf = WarpWorkflow::new();
        wf.start(conversation(1), Instant::now());
        assert_eq!(
            wf.take(&conversation(1), Duration::from_secs(60)),
            WarpFlow::Waiting
        );
        assert_eq!(
            wf.take(&conversation(1), Duration::from_secs(60)),
            WarpFlow::Continue
        );
    }

    #[test]
    fn expiry_boundary_is_table_driven() {
        let stale = Instant::now() - Duration::from_secs(60);
        let cases = [
            (Duration::from_secs(59), WarpFlow::Expired),
            (Duration::from_secs(60), WarpFlow::Expired),
            (Duration::from_secs(61), WarpFlow::Waiting),
        ];
        for (timeout, expected) in cases {
            let wf = WarpWorkflow::new();
            wf.start(conversation(1), stale);
            assert_eq!(wf.take(&conversation(1), timeout), expected);
        }
    }
}
