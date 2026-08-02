use crate::core::i18n::Lang;

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
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
}

#[mockall::automock]
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

    async fn answer_callback(
        &self,
        _target: &TargetId,
        _callback_id: &str,
        _text: Option<String>,
    ) -> Result<()> {
        Ok(())
    }

    async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
        anyhow::bail!("platform does not support file download")
    }

    async fn set_system_locale(&self, _lang: Lang) -> Result<()> {
        Ok(())
    }

    async fn send_file(
        &self,
        _target: &TargetId,
        _name: &str,
        _data: Vec<u8>,
        _mime: &str,
    ) -> Result<MessageId> {
        anyhow::bail!("platform does not support file sending")
    }

    async fn send_image(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.send_file(target, "image", data, mime).await
    }

    async fn send_voice(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.send_file(target, "voice", data, mime).await
    }

    async fn send_typing(&self, _target: &TargetId, _active: bool) -> Result<()> {
        Ok(())
    }

    async fn send_reaction(
        &self,
        _target: &TargetId,
        _msg_id: &MessageId,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn send_message_threaded(
        &self,
        target: &TargetId,
        content: MessageContent,
        _thread_root: &str,
    ) -> Result<MessageId> {
        self.send_message(target, content).await
    }
}
