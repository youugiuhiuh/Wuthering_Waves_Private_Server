use crate::adapters::common::routing::is_sensitive;
use crate::adapters::common::{
    BotAdapter, Markup, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use anyhow::Result;
use async_trait::async_trait;
use matrix_sdk::attachment::AttachmentConfig;
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

/// Render inline keyboard markup as a text command list for Matrix clients
/// that don't support inline keyboards.
fn render_markup_buttons(base: String, markup: &Markup) -> String {
    let mut body = base;
    let mut lines: Vec<String> = Vec::new();
    let mut idx = 1;
    for row in &markup.buttons {
        for btn in row {
            lines.push(format!("{}. {} — send: `{}`", idx, btn.text, btn.data));
            idx += 1;
        }
    }
    if !lines.is_empty() {
        body.push_str(&rust_i18n::t!("matrix.markup_header"));
        body.push_str(&lines.join("\n"));
    }
    body
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

#[cfg(test)]
mod matrix_adapter_tests {
    use crate::adapters::common::{InlineButton, Markup};

    #[test]
    fn send_message_with_markup_appends_command_list() {
        let markup = Markup {
            buttons: vec![
                vec![
                    InlineButton {
                        text: "Search".into(),
                        data: "/search".into(),
                    },
                    InlineButton {
                        text: "Help".into(),
                        data: "/help".into(),
                    },
                ],
                vec![InlineButton {
                    text: "Cancel".into(),
                    data: "/cancel".into(),
                }],
            ],
        };
        let result = super::render_markup_buttons("Hello".to_string(), &markup);
        assert!(result.contains("Hello"));
        assert!(result.contains("1. Search"));
        assert!(result.contains("/search"));
        assert!(result.contains("2. Help"));
        assert!(result.contains("/help"));
        assert!(result.contains("3. Cancel"));
        assert!(result.contains("/cancel"));
    }

    #[test]
    fn send_message_without_markup_returns_plain_text() {
        let result = super::render_markup_buttons("plain".into(), &Markup { buttons: vec![] });
        assert_eq!(result, "plain");
    }
}

#[async_trait]
impl BotAdapter for MatrixAdapter {
    fn platform(&self) -> Platform {
        Platform::Matrix
    }

    async fn send_message(&self, _target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let body_text = match &content.markup {
            Some(markup) => render_markup_buttons(content.text, markup),
            None => content.text,
        };

        if is_sensitive(&body_text) {
            let data = body_text.into_bytes();
            let response = self
                .room
                .send_attachment(
                    "batch_result.txt",
                    &mime::TEXT_PLAIN,
                    data,
                    AttachmentConfig::new(),
                )
                .await?;
            Ok(MessageId(response.event_id.to_string()))
        } else {
            let body = RoomMessageEventContent::text_plain(&body_text);
            let response = self.room.send(body).await?;
            Ok(MessageId(response.response.event_id.to_string()))
        }
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

    async fn answer_callback(
        &self,
        _target: &TargetId,
        _callback_id: &str,
        _text: Option<String>,
    ) -> Result<()> {
        Ok(())
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        use matrix_sdk::media::MediaRequestParameters;
        use matrix_sdk::ruma::OwnedMxcUri;
        use matrix_sdk::ruma::events::room::MediaSource;
        let client = self.room.client();
        let mxc = OwnedMxcUri::from(file_id.to_string());
        let request = MediaRequestParameters {
            source: MediaSource::Plain(mxc),
            format: matrix_sdk::media::MediaFormat::File,
        };
        let data = client.media().get_media_content(&request, false).await?;
        Ok(data)
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            can_edit_message: true,
            can_delete_message: true,
            has_inline_keyboard: false,
            has_slash_commands: false,
            has_file_transfer: true,
        }
    }
}
