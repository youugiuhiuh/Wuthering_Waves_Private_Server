use crate::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
use anyhow::Result;
use async_trait::async_trait;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::OwnedEventId;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

/// Matrix adapter: sends messages to a single Matrix room.
///
/// One `MatrixAdapter` instance maps to one room. For multiple rooms,
/// create multiple instances. The `target` parameter in `BotAdapter`
/// is ignored since the room is fixed at construction time.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_is_matrix() {
        // Platform() doesn't depend on Room — verify the constant
        assert_eq!(Platform::Matrix as u8, 2);
    }

    #[test]
    fn message_id_roundtrip_valid() {
        let mid = MessageId("$valid_event_id:matrix.org".to_string());
        let parsed: std::result::Result<OwnedEventId, _> = mid.0.parse();
        assert!(parsed.is_ok());
    }

    #[test]
    fn message_id_parse_rejects_invalid() {
        let mid = MessageId("not a valid event id".to_string());
        let parsed: std::result::Result<OwnedEventId, _> = mid.0.parse();
        assert!(parsed.is_err());
    }

    #[test]
    fn platform_enum_value() {
        assert_eq!(Platform::Matrix, Platform::Matrix);
        assert_ne!(Platform::Matrix, Platform::Telegram);
        assert_ne!(Platform::Matrix, Platform::Discord);
    }
}

#[async_trait]
impl BotAdapter for MatrixAdapter {
    fn platform(&self) -> Platform {
        Platform::Matrix
    }

    async fn send_message(&self, _target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let body = RoomMessageEventContent::text_html(&content.text, &content.text);
        let response = self.room.send(body).await?;
        Ok(MessageId(response.event_id.to_string()))
    }

    async fn edit_message(
        &self,
        _target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let event_id: OwnedEventId = msg_id
            .0
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        let new_content = RoomMessageEventContent::text_html(&content.text, &content.text)
            .make_replacement(
                matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(event_id, None),
                None,
            );
        self.room.send(new_content).await?;
        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let event_id: OwnedEventId = msg_id
            .0
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        self.room.redact(&event_id, None, None).await?;
        Ok(())
    }
}
