use crate::common::markup::render_markup_buttons;
use crate::common::routing::is_sensitive;
use crate::common::{
    BotAdapter, Markup, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
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

    /// poll 发不出去时的兜底：把按钮降级成正文里的编号命令清单，保证菜单仍可操作。
    /// 发 `poll.end` 结束一条 poll（取代 redact：redact 在 Element 里留「消息已被删除」墓碑）。
    async fn end_poll(&self, poll_id: &str) {
        match crate::gateways::matrix::poll::poll_end_content(poll_id) {
            Some(end) => {
                if let Err(e) = self.room.send(end).await {
                    log::warn!("结束旧 poll 失败（不影响后续）: {e}");
                }
            }
            None => log::warn!("poll event id 无法解析，跳过 poll.end: {poll_id}"),
        }
    }

    async fn send_command_list_fallback(&self, text: &str, markup: &Option<Markup>, reason: &str) {
        log::warn!("Matrix poll {reason}，回退为编号命令清单");
        if let Some(m) = markup {
            let body = render_markup_buttons(text.to_string(), m);
            if let Err(e) = self
                .room
                .send(RoomMessageEventContent::text_plain(&body))
                .await
            {
                log::warn!("回退编号清单也发送失败: {e}");
            }
        }
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
                    body_text.clone().into_bytes(),
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
            // plan_send 已过滤 0 与 >MAX_POLL_ANSWERS，此处 None 属防御性分支。
            match self.room.send(poll_content).await {
                Ok(resp) => {
                    // 新消息，同一 anchor 不应有旧 poll；有就结束它。
                    let displaced = self.polls.insert(
                        resp.response.event_id.to_string(),
                        PollBinding::new(MessageId(anchor.clone()), buttons),
                    );
                    if let Some(old) = displaced {
                        self.end_poll(&old).await;
                    }
                }
                Err(e) => {
                    self.send_command_list_fallback(&body_text, &content.markup, "发送失败")
                        .await;
                    log::warn!("Matrix poll 发送失败: {e}");
                }
            }
        }

        Ok(MessageId(anchor))
    }

    fn take_poll_answer(&self, poll_id: &str, index: usize) -> Option<(MessageId, String)> {
        self.polls.take_unanswered(poll_id, index)
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
        let (body_text, poll_spec) = crate::gateways::matrix::poll::plan_send(&content);
        let new_content = RoomMessageEventContent::text_html(&body_text, &body_text)
            .make_replacement(
                matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(event_id, None),
            );
        self.room.send(new_content).await?;

        // 先发新 poll 再结束旧 poll：新 poll 发失败时旧 poll 仍在时间线（也在映射表）里、仍可点。
        let has_poll = poll_spec.is_some();
        if let Some(spec) = poll_spec
            && let Some(poll_content) = crate::gateways::matrix::poll::poll_start_content(&spec)
        {
            // plan_send 已过滤 0 与 >MAX_POLL_ANSWERS，此处 None 属防御性分支。
            match self.room.send(poll_content).await {
                Ok(resp) => {
                    // `insert` 自己就地把同 anchor 的旧 binding 置掉并交回其 poll id。
                    // 绝不在这里按 anchor 反查旧 poll：HashMap 迭代顺序任意，会搞到刚发的这个。
                    let displaced = self.polls.insert(
                        resp.response.event_id.to_string(),
                        PollBinding::new(
                            msg_id.clone(),
                            crate::gateways::matrix::poll::flatten(&content.markup),
                        ),
                    );
                    if let Some(old) = displaced {
                        self.end_poll(&old).await;
                    }
                }
                Err(e) => {
                    self.send_command_list_fallback(&body_text, &content.markup, "发送失败")
                        .await;
                    log::warn!("Matrix poll 发送失败: {e}");
                }
            }
        }

        // 菜单变成非菜单消息（无 spec）：这条 anchor 名下的旧 poll 必须结束。
        // 有 spec 时已由上面 insert 的置换完成，此处不得再按 anchor 反查。
        if !has_poll && let Some(old_poll) = self.polls.take_by_anchor(&msg_id.0) {
            self.end_poll(&old_poll).await;
        }

        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let event_id: OwnedEventId = msg_id
            .0
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        // poll 是独立事件，redact 正文带不走它；用 poll.end 结束它，
        // 否则房间里会留下点不动的幽灵选项。
        if let Some(poll_id) = self.polls.take_by_anchor(&msg_id.0) {
            self.end_poll(&poll_id).await;
        }
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
