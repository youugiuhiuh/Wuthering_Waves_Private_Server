use anyhow::Result;
use futures_util::future::BoxFuture;
use std::sync::Arc;

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

pub async fn execute_supervised(executor: Arc<dyn SelfDestructExecutor>) -> Result<()> {
    executor.execute().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
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
    async fn test_execute_supervised_returns_ok() {
        let mut mock = MockExecutorMock::new();
        mock.expect_execute()
            .times(1)
            .returning(|| Box::pin(async { Ok(()) }));

        let result = execute_supervised(Arc::new(mock)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_supervised_propagates_error() {
        let mut mock = MockExecutorMock::new();
        mock.expect_execute()
            .times(1)
            .returning(|| Box::pin(async { Err(anyhow::anyhow!("test error")) }));

        let result = execute_supervised(Arc::new(mock)).await;
        assert!(result.is_err());
    }
}
