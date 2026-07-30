use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone)]
pub enum Component {
    System,
    Scheduler,
    Upgrade,
    Security,
}

#[derive(Debug, Clone)]
pub enum Status {
    Started,
    Stopped,
    Healthy,
    Degraded,
    Failed,
}

#[derive(Debug, Clone)]
pub enum CoreEvent {
    Alert {
        severity: Severity,
        title: String,
        body: String,
    },
    StatusChange {
        component: Component,
        status: Status,
    },
    Scheduled {
        task_name: String,
        payload: String,
    },
}

#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CoreEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: CoreEvent) {
        let _ = self.tx.send(event);
    }
}
