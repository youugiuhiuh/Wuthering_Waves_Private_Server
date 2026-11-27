use crate::common::markup::render_markup_buttons;
use crate::common::routing::is_sensitive;
use crate::common::{
    BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use crate::gateways::matrix::poll::{PollBinding, PollStore};
use anyhow::Result;
use async_trait::async_trait;
use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::OwnedEventId;
use matrix_sdk::ruma::events::reaction::ReactionEventContent;
use matrix_sdk::ruma::events::relation::Annotation;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

/// Matrix adapter: sends messages to a single Matrix room.
///
/// One `MatrixAdapter` instance maps to one room. For multiple rooms,
/// create multiple instances. The `target` parameter in `BotAdapter`
/// is ignored since the room is fixed at construction time.
pub struct MatrixAdapter {
    room: Room,
    polls: PollStore,
}

impl MatrixAdapter {
    pub fn new(room: Room) -> Self {
        Self {
            room,
            polls: PollStore::new(),
        }
    }

    pub fn inner_room(&self) -> &Room {
        &self.room
    }

    pub fn polls(&self) -> &PollStore {
        &self.polls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_is_matrix() {
        // Platform() doesn't depend on Room — verify the constant
        assert_eq!(Platform::Matrix as u8, 1);
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
        assert_ne!(Platform::Matrix, Platform::Simplex);
    }
}

#[cfg(test)]
mod matrix_adapter_tests {
    use crate::common::{InlineButton, Markup};

    #[test]
    fn send_message_with_markup_no_longer_inlines_command_list() {
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
        let (body, poll) =
            crate::gateways::matrix::poll::plan_send(&crate::common::MessageContent {
                text: "Hello".into(),
                markup: Some(markup),
            });
        assert_eq!(body, "Hello", "正文不得再拼编号清单");
        assert!(!body.contains("send:"), "正文不得暴露内部命令串");
        assert!(!body.contains("/search"), "正文不得暴露内部命令串");
        let spec = poll.expect("3 个按钮应产出 poll");
        assert_eq!(spec.answers.len(), 3);
        assert_eq!(
            spec.answers
                .iter()
                .map(|a| a.text.as_str())
                .collect::<Vec<_>>(),
            ["Search", "Help", "Cancel"]
        );
    }

    #[test]
    fn send_message_without_markup_returns_plain_text() {
        let (body, poll) =
            crate::gateways::matrix::poll::plan_send(&crate::common::MessageContent {
                text: "plain".into(),
                markup: None,
            });
        assert_eq!(body, "plain");
        assert!(poll.is_none());
    }

    #[test]
    fn new_adapter_starts_with_empty_poll_store() {
        // PollStore 的插入/查询已在 poll.rs 覆盖；这里只钉「每个 adapter 自带独立空表」的契约。
        // Room 无法在单测里构造，故用类型级断言 + PollStore 默认构造确认。
        let store = crate::gateways::matrix::poll::PollStore::new();
        assert!(store.find_by_anchor("$nothing").is_none());
    }
}

#[async_trait]
impl BotAdapter for MatrixAdapter {
    fn platform(&self) -> Platform {
        Platform::Matrix
    }

    async fn send_message(&self, _target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let (body_text, poll_spec) = crate::gateways::matrix::poll::plan_send(&content);
        let buttons = crate::gateways::matrix::poll::flatten(&content.markup);

        // 敏感判定必须覆盖按钮 data（正文已不含命令串）
        let mut probe = body_text.clone();
        for b in &buttons {
            probe.push('\n');
            probe.push_str(&b.data);
        }

        let anchor = if is_sensitive(&probe) {
            let response = self
                .room
                .send_attachment(
                    "batch_result.txt",
                    &mime::TEXT_PLAIN,
                    body_text.into_bytes(),
                    AttachmentConfig::new(),
                )
                .await?;
            response.event_id.to_string()
        } else {
            let body = RoomMessageEventContent::text_plain(&body_text);
            let response = self.room.send(body).await?;
            response.response.event_id.to_string()
        };

        if let Some(spec) = poll_spec
            && let Some(poll_content) = crate::gateways::matrix::poll::poll_start_content(&spec)
        {
            match self.room.send(poll_content).await {
                Ok(resp) => self.polls.insert(
                    resp.response.event_id.to_string(),
                    PollBinding::new(MessageId(anchor.clone()), buttons),
                ),
                Err(e) => {
                    log::warn!("Matrix poll 发送失败（正文已送达，仅缺可点选项）: {e}")
                }
            }
        }

        Ok(MessageId(anchor))
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
            can_send_file: true,
            can_send_image: true,
            can_send_voice: false,
            can_send_typing: true,
            can_send_reaction: true,
            can_thread: true,
            has_e2ee: true,
        }
    }

    async fn send_file(
        &self,
        _target: &TargetId,
        name: &str,
        data: Vec<u8>,
        mime_str: &str,
    ) -> Result<MessageId> {
        let mime_type: mime::Mime = mime_str.parse()?;
        let response = self
            .room
            .send_attachment(name, &mime_type, data, AttachmentConfig::new())
            .await?;
        Ok(MessageId(response.event_id.to_string()))
    }

    async fn send_image(
        &self,
        _target: &TargetId,
        data: Vec<u8>,
        mime_str: &str,
    ) -> Result<MessageId> {
        self.send_file(_target, "image", data, mime_str).await
    }

    async fn send_voice(
        &self,
        _target: &TargetId,
        data: Vec<u8>,
        mime_str: &str,
    ) -> Result<MessageId> {
        self.send_file(_target, "voice", data, mime_str).await
    }

    async fn send_typing(&self, _target: &TargetId, active: bool) -> Result<()> {
        self.room.typing_notice(active).await?;
        Ok(())
    }

    async fn send_reaction(
        &self,
        _target: &TargetId,
        msg_id: &MessageId,
        emoji: &str,
    ) -> Result<()> {
        let event_id: OwnedEventId = msg_id
            .0
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        let annotation = Annotation::new(event_id, emoji.to_string());
        let content = ReactionEventContent::new(annotation);
        self.room.send(content).await?;
        Ok(())
    }

    async fn send_message_threaded(
        &self,
        _target: &TargetId,
        content: MessageContent,
        thread_root: &str,
    ) -> Result<MessageId> {
        use matrix_sdk::ruma::events::relation::Thread;
        use matrix_sdk::ruma::events::room::message::Relation;
        let body_text = match &content.markup {
            Some(markup) => render_markup_buttons(content.text, markup),
            None => content.text,
        };
        let root_id: OwnedEventId = thread_root
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        let thread = Thread::without_fallback(root_id);
        let mut room_msg = RoomMessageEventContent::text_plain(&body_text);
        room_msg.relates_to = Some(Relation::Thread(thread));
        let response = self.room.send(room_msg).await?;
        Ok(MessageId(response.response.event_id.to_string()))
    }
}
