use crate::app::interaction::BusinessMessage;
use async_trait::async_trait;

#[async_trait]
pub trait BusinessOutput: Send + Sync {
    async fn publish(&self, message: BusinessMessage) -> anyhow::Result<()>;
}
