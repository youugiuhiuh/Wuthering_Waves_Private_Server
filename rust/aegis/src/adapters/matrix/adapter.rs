use crate::adapters::common::{
    BotAdapter, MessageContent, MessageId, Platform, TargetId,
};
use anyhow::Result;
use async_trait::async_trait;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::OwnedEventId;
use matrix_sdk::Room;

pub struct MatrixAdapter {
    room: Room,
}

impl MatrixAdapter {
    pub fn new(room: Room) -> Self {
        Self { room }
    }

    pub fn inner_room(&self) -> &Room {
        &self.room
    }
}

#[async_trait]
impl BotAdapter for MatrixAdapter {
    fn platform(&self) -> Platform {
        Platform::Matrix
    }

    async fn send_message(&self, _target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let response = self
            .room
            .send(RoomMessageEventContent::text_html(
                &content.text,
                &content.text,
            ))
            .await?;
        Ok(MessageId(response.event_id.to_string()))
    }

    async fn edit_message(
        &self,
        _target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let event_id: OwnedEventId = msg_id.0.parse()?;
        let new_content =
            RoomMessageEventContent::text_html(&content.text, &content.text);
        let edit = new_content.make_replacement(
            matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(event_id, None),
            None,
        );
        self.room.send(edit).await?;
        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let event_id: OwnedEventId = msg_id.0.parse()?;
        self.room.redact(&event_id, None, None).await?;
        Ok(())
    }
}
