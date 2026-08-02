use crate::app::interaction::OutputAction;
use async_trait::async_trait;

#[async_trait]
pub trait BusinessOutput: Send + Sync {
    async fn publish(&self, action: OutputAction) -> anyhow::Result<()>;
}
