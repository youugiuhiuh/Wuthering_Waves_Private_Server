use anyhow::Result;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

use crate::core::system::maintenance::MaintenanceManager;

pub trait SelfDestructExecutor: Send + Sync {
    fn execute(&self) -> BoxFuture<'static, Result<()>>;
}

pub struct ProductionSelfDestructExecutor;

impl SelfDestructExecutor for ProductionSelfDestructExecutor {
    fn execute(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { MaintenanceManager::perform_self_destruct().await })
    }
}

pub fn production_executor() -> Arc<dyn SelfDestructExecutor> {
    Arc::new(ProductionSelfDestructExecutor)
}

pub fn trigger(executor: Arc<dyn SelfDestructExecutor>) {
    tokio::task::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(err) = executor.execute().await {
            eprintln!("Self destruct failed: {}", err);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use std::time::Duration;

    mock! {
        pub ExecutorMock {}
        impl SelfDestructExecutor for ExecutorMock {
            fn execute(&self) -> BoxFuture<'static, Result<()>>;
        }
    }

    #[test]
    fn test_production_executor_creation() {
        let executor = production_executor();
        assert!(Arc::strong_count(&executor) >= 1);
    }

    #[test]
    fn test_executor_trait_available() {
        let executor: Arc<dyn SelfDestructExecutor> = production_executor();
        let _ = executor.clone();
    }

    #[tokio::test]
    async fn test_trigger_calls_executor() {
        let mut mock = MockExecutorMock::new();
        mock.expect_execute()
            .times(1)
            .returning(|| Box::pin(async { Ok(()) }));

        trigger(Arc::new(mock));
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    #[tokio::test]
    async fn test_trigger_handles_executor_error() {
        let mut mock = MockExecutorMock::new();
        mock.expect_execute()
            .times(1)
            .returning(|| Box::pin(async { Err(anyhow::anyhow!("test error")) }));

        trigger(Arc::new(mock));
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
