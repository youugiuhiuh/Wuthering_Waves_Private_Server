use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct TargetId(pub String);

#[derive(Debug, Clone)]
pub struct MessageId(pub String);

#[derive(Debug, Clone)]
pub struct MessageContent {
    pub text: String,
    pub markup: Option<Markup>,
}

#[derive(Debug, Clone)]
pub struct Markup {
    pub buttons: Vec<Vec<InlineButton>>,
}

#[derive(Debug, Clone)]
pub struct InlineButton {
    pub text: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
}

#[async_trait]
pub trait BotAdapter: Send + Sync {
    fn platform(&self) -> Platform;
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;
    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()>;
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;
}
