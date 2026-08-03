use crate::app::interaction::OutputAction;
use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait BusinessOutput: Send + Sync {
    async fn publish(&self, action: OutputAction) -> anyhow::Result<()>;
    fn as_adapter(&self) -> Arc<dyn BotAdapter>;
}

pub struct NoopBotAdapter;

impl NoopBotAdapter {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<dyn BotAdapter> {
        Arc::new(NoopBotAdapter)
    }
}

#[async_trait]
impl BotAdapter for NoopBotAdapter {
    fn platform(&self) -> Platform {
        Platform::Telegram
    }

    async fn send_message(
        &self,
        _target: &TargetId,
        _content: MessageContent,
    ) -> anyhow::Result<MessageId> {
        Ok(MessageId("noop".to_string()))
    }

    async fn edit_message(
        &self,
        _target: &TargetId,
        _msg_id: &MessageId,
        _content: MessageContent,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, _msg_id: &MessageId) -> anyhow::Result<()> {
        Ok(())
    }
}
