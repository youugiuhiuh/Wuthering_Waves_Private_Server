use anyhow::Result;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

use crate::logic::maintenance::MaintenanceManager;

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
