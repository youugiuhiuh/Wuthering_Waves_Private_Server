use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::Result;
use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

pub struct TelegramPresenter {
    bot: Bot,
}

impl TelegramPresenter {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

#[async_trait]
impl BusinessOutput for TelegramPresenter {
    async fn publish(&self, action: OutputAction) -> Result<()> {
        match action {
            OutputAction::SendText {
                target_conversation,
                payload,
                sensitivity,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
                match sensitivity {
                    Sensitivity::Protected => {
                        let text = match payload {
                            OutputPayload::Text { text } => text,
                            OutputPayload::Attachment {
                                bytes,
                                filename: _,
                                mime: _,
                            } => String::from_utf8_lossy(&bytes).to_string(),
                        };
                        self.bot
                            .send_document(
                                chat_id,
                                teloxide::types::InputFile::memory(text.into_bytes()),
                            )
                            .await?;
                    }
                    Sensitivity::Public => match payload {
                        OutputPayload::Text { text } => {
                            self.bot
                                .send_message(chat_id, &text)
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                        OutputPayload::Attachment {
                            bytes,
                            filename: _,
                            mime: _,
                        } => {
                            self.bot
                                .send_document(chat_id, teloxide::types::InputFile::memory(bytes))
                                .await?;
                        }
                    },
                }
            }
            OutputAction::SendAttachment {
                target_conversation,
                payload,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
                if let OutputPayload::Attachment {
                    bytes,
                    filename: _,
                    mime: _,
                } = payload
                {
                    self.bot
                        .send_document(chat_id, teloxide::types::InputFile::memory(bytes))
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
