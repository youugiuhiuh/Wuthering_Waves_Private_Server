#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationProgress {
    Started(String),
    Advanced(String),
    Finished(String),
}

#[async_trait::async_trait]
pub trait ProgressReporter: Send + Sync {
    async fn report(&self, progress: OperationProgress) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default, Clone)]
    struct RecordingReporter(Arc<Mutex<Vec<OperationProgress>>>);

    #[async_trait::async_trait]
    impl ProgressReporter for RecordingReporter {
        async fn report(&self, progress: OperationProgress) -> anyhow::Result<()> {
            self.0.lock().await.push(progress);
            Ok(())
        }
    }

    impl RecordingReporter {
        async fn events(&self) -> Vec<OperationProgress> {
            self.0.lock().await.clone()
        }
    }

    #[tokio::test]
    async fn recording_reporter_receives_ordered_updates() {
        let reporter = RecordingReporter::default();
        reporter
            .report(OperationProgress::Started("开始".into()))
            .await
            .unwrap();
        reporter
            .report(OperationProgress::Advanced("进行中".into()))
            .await
            .unwrap();
        reporter
            .report(OperationProgress::Finished("完成".into()))
            .await
            .unwrap();
        let events = reporter.events().await;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], OperationProgress::Started("开始".into()));
        assert_eq!(events[1], OperationProgress::Advanced("进行中".into()));
        assert_eq!(events[2], OperationProgress::Finished("完成".into()));
    }
}
