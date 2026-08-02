use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serenity::all::{ChannelId, CreateAttachment};
use serenity::http::Http;
use std::sync::Arc;

pub struct DiscordPresenter {
    http: Arc<Http>,
}

impl DiscordPresenter {
    pub fn new(http: Arc<Http>) -> Self {
        Self { http }
    }
}

#[async_trait]
impl BusinessOutput for DiscordPresenter {
    async fn publish(&self, action: OutputAction) -> Result<()> {
        match action {
            OutputAction::SendText {
                target_conversation,
                payload,
                sensitivity,
            } => {
                let channel_id = ChannelId::new(
                    target_conversation
                        .as_str()
                        .parse::<u64>()
                        .context("invalid channel id")?,
                );
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
                        let msg = channel_id
                            .send_message(
                                &self.http,
                                serenity::all::CreateMessage::new().add_file(
                                    CreateAttachment::bytes(text.into_bytes(), "message.txt"),
                                ),
                            )
                            .await
                            .context("发送 Discord 文件消息失败")?;
                        let _ = msg;
                    }
                    Sensitivity::Public => {
                        let text = match payload {
                            OutputPayload::Text { text } => text,
                            OutputPayload::Attachment {
                                bytes,
                                filename: _,
                                mime: _,
                            } => String::from_utf8_lossy(&bytes).to_string(),
                        };
                        let msg = channel_id
                            .send_message(
                                &self.http,
                                serenity::all::CreateMessage::new().content(&text),
                            )
                            .await
                            .context("发送 Discord 消息失败")?;
                        let _ = msg;
                    }
                }
            }
            OutputAction::SendAttachment {
                target_conversation,
                payload,
            } => {
                let channel_id = ChannelId::new(
                    target_conversation
                        .as_str()
                        .parse::<u64>()
                        .context("invalid channel id")?,
                );
                if let OutputPayload::Attachment {
                    bytes,
                    filename,
                    mime: _,
                } = payload
                {
                    let msg = channel_id
                        .send_message(
                            &self.http,
                            serenity::all::CreateMessage::new()
                                .add_file(CreateAttachment::bytes(bytes, &filename)),
                        )
                        .await
                        .context("发送 Discord 附件失败")?;
                    let _ = msg;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
