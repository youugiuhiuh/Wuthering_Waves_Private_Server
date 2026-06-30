use tokio::sync::broadcast;

/// Severity level of a [`CoreEvent`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// System component that generated a [`CoreEvent`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Component {
    System,
    Scheduler,
    Upgrade,
    Security,
}

/// Health status of a system component.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Status {
    Started,
    Stopped,
    Healthy,
    Degraded,
    Failed,
}

/// Core system event published via the event bus.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CoreEvent {
    /// An alert with severity, title, and body.
    Alert {
        severity: Severity,
        title: String,
        body: String,
    },
    /// A component status transition.
    StatusChange {
        component: Component,
        status: Status,
    },
    /// A scheduled task trigger.
    Scheduled { task_name: String, payload: String },
}

/// In-process event bus using a broadcast channel.
///
/// Emitted [`CoreEvent`]s are delivered to all subscribers.
/// The channel capacity is fixed at creation time.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CoreEvent>,
}

impl EventBus {
    /// Creates a new event bus with the given channel capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// # use aegis::core::events::EventBus;
    /// let bus = EventBus::new(64);
    /// ```
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribes to all future [`CoreEvent`]s.
    ///
    /// Returns a receiver that may lag if the subscriber is slower
    /// than the producer and the channel capacity is exceeded.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.tx.subscribe()
    }

    /// Emits an event to all subscribers.
    ///
    /// If all receivers have been dropped the event is silently discarded
    /// (the broadcast send error is deliberately ignored).
    pub fn emit(&self, event: CoreEvent) {
        let _ = self.tx.send(event);
    }
}
