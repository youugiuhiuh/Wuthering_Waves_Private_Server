use crate::app::interaction::{BusinessMessage, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::Result;
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

#[async_trait]
impl BusinessOutput for MatrixPresenter {
    async fn publish(&self, message: BusinessMessage) -> Result<()> {
        match message.sensitivity {
            Sensitivity::Protected => {
                let data = message.text.into_bytes();
                self.room
                    .send_attachment(
                        "message.txt",
                        &mime::TEXT_PLAIN,
                        data,
                        AttachmentConfig::new(),
                    )
                    .await?;
            }
            Sensitivity::Public => {
                let content = RoomMessageEventContent::text_plain(&message.text);
                self.room.send(content).await?;
            }
        }
        Ok(())
    }
}
