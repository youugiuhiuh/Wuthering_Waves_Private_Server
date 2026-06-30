use anyhow::Result;
use async_trait::async_trait;

/// Wrapper for a platform-specific chat target identifier.
#[derive(Debug, Clone)]
pub struct TargetId(pub String);

/// Wrapper for a platform-specific message identifier.
#[derive(Debug, Clone)]
pub struct MessageId(pub String);

/// Content to send as a bot message.
#[derive(Debug, Clone)]
pub struct MessageContent {
    /// The message body text.
    pub text: String,
    /// Optional inline keyboard markup.
    pub markup: Option<Markup>,
}

/// Inline keyboard definition.
#[derive(Debug, Clone)]
pub struct Markup {
    /// Rows of inline buttons.
    pub buttons: Vec<Vec<InlineButton>>,
}

/// A single inline keyboard button.
#[derive(Debug, Clone)]
pub struct InlineButton {
    /// Button label displayed to the user.
    pub text: String,
    /// Callback data sent when the button is pressed.
    pub data: String,
}

/// Supported chat platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
}

/// Platform-agnostic bot adapter trait.
///
/// Implementations provide a uniform interface over Telegram, Discord,
/// and Matrix bots. All handler code targets this trait.
#[async_trait]
pub trait BotAdapter: Send + Sync {
    /// Returns the platform this adapter is connected to.
    fn platform(&self) -> Platform;

    /// Sends a message to the given target and returns the assigned message ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying platform fails to send
    /// (e.g. network error, rate-limit, or invalid target).
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;

    /// Replaces the content of an existing message.
    ///
    /// # Errors
    ///
    /// Returns an error if the message does not exist, was deleted,
    /// or the platform request fails.
    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()>;

    /// Deletes a previously sent message.
    ///
    /// # Errors
    ///
    /// Returns an error if the message does not exist or the platform
    /// request fails.
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;
}
