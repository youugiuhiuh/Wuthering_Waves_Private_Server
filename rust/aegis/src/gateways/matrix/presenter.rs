use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

pub struct MatrixPresenter {
    room: Room,
}

impl MatrixPresenter {
    pub fn new(room: Room) -> Self {
        Self { room }
    }
}

fn parse_mime(s: &str) -> Result<mime::Mime> {
    s.parse().map_err(|_| anyhow!("invalid mime type: {}", s))
}

#[async_trait]
impl BusinessOutput for MatrixPresenter {
    async fn publish(&self, action: OutputAction) -> Result<()> {
        match action {
            OutputAction::SendText {
                target_conversation: _,
                payload,
                sensitivity,
            } => match sensitivity {
                Sensitivity::Protected => {
                    let (data, mime_val) = match payload {
                        OutputPayload::Text { text } => (text.into_bytes(), mime::TEXT_PLAIN),
                        OutputPayload::Attachment {
                            bytes,
                            filename: _,
                            mime,
                        } => (bytes, parse_mime(&mime)?),
                    };
                    self.room
                        .send_attachment("message.txt", &mime_val, data, AttachmentConfig::new())
                        .await?;
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
                    let content = RoomMessageEventContent::text_plain(&text);
                    self.room.send(content).await?;
                }
            },
            OutputAction::SendAttachment {
                target_conversation: _,
                payload:
                    OutputPayload::Attachment {
                        bytes,
                        filename,
                        mime,
                    },
            } => {
                let mime_val = parse_mime(&mime)?;
                self.room
                    .send_attachment(&filename, &mime_val, bytes, AttachmentConfig::new())
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}
