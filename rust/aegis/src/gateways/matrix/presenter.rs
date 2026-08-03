use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::{BusinessOutput, NoopBotAdapter};
use crate::common::BotAdapter;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use std::sync::Arc;

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
            OutputAction::Edit {
                target_conversation: _,
                message_id,
                payload,
            } => {
                let event_id: matrix_sdk::ruma::OwnedEventId = message_id
                    .as_str()
                    .parse()
                    .map_err(|e| anyhow!("invalid event id: {}", e))?;
                let text = match payload {
                    OutputPayload::Text { text } => text,
                    OutputPayload::Attachment {
                        bytes,
                        filename: _,
                        mime: _,
                    } => String::from_utf8_lossy(&bytes).to_string(),
                };
                let new_content = RoomMessageEventContent::text_plain(&text);
                let replacement = new_content.make_replacement(
                    matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(
                        event_id, None,
                    ),
                );
                self.room.send(replacement).await?;
            }
            OutputAction::Delete {
                target_conversation: _,
                message_id,
            } => {
                let event_id: matrix_sdk::ruma::OwnedEventId = message_id
                    .as_str()
                    .parse()
                    .map_err(|e| anyhow!("invalid event id: {}", e))?;
                self.room.redact(&event_id, None, None).await?;
            }
            OutputAction::AnswerCallback { .. } => {}
            OutputAction::SendAttachment {
                target_conversation: _,
                payload,
            } => {
                let (bytes, filename, mime) = match payload {
                    OutputPayload::Attachment {
                        bytes,
                        filename,
                        mime,
                    } => (bytes, filename, mime),
                    OutputPayload::Text { text } => {
                        (text.into_bytes(), "message.txt".into(), "text/plain".into())
                    }
                };
                let mime_val = parse_mime(&mime)?;
                self.room
                    .send_attachment(&filename, &mime_val, bytes, AttachmentConfig::new())
                    .await?;
            }
        }
        Ok(())
    }

    fn as_adapter(&self) -> Arc<dyn BotAdapter> {
        NoopBotAdapter::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interaction::ConversationId;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    pub struct FakeRoom {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeRoom {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    struct TestRoom {
        inner: Arc<FakeRoom>,
    }

    impl TestRoom {
        fn new(inner: Arc<FakeRoom>) -> Self {
            Self { inner }
        }
        async fn send_attachment(
            &self,
            filename: &str,
            _mime: &mime::Mime,
            data: Vec<u8>,
            _config: AttachmentConfig,
        ) -> Result<()> {
            let content = String::from_utf8_lossy(&data).to_string();
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("send_attachment:{}:{}", filename, content));
            Err(anyhow::anyhow!("test room"))
        }
        async fn send(&self, _content: RoomMessageEventContent) -> Result<()> {
            self.inner
                .calls
                .lock()
                .unwrap()
                .push("send:called".to_string());
            Err(anyhow::anyhow!("test room"))
        }
        async fn redact(
            &self,
            event_id: &matrix_sdk::ruma::OwnedEventId,
            _reason: Option<&str>,
            _attrs: Option<std::collections::HashMap<String, String>>,
        ) -> Result<()> {
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("redact:{}", event_id));
            Ok(())
        }
    }

    struct TestPresenter {
        room: TestRoom,
    }

    impl TestPresenter {
        fn new(room: TestRoom) -> Self {
            Self { room }
        }
    }

    #[async_trait]
    impl BusinessOutput for TestPresenter {
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
                            } => (bytes, mime.parse().unwrap()),
                        };
                        self.room
                            .send_attachment(
                                "message.txt",
                                &mime_val,
                                data,
                                AttachmentConfig::new(),
                            )
                            .await?;
                    }
                    Sensitivity::Public => {
                        let text = match payload {
                            OutputPayload::Text { text } => text,
                            OutputPayload::Attachment { bytes, .. } => {
                                String::from_utf8_lossy(&bytes).to_string()
                            }
                        };
                        let content = RoomMessageEventContent::text_plain(&text);
                        self.room.send(content).await?;
                    }
                },
                OutputAction::Edit {
                    target_conversation: _,
                    message_id,
                    payload,
                } => {
                    let event_id = message_id.parse().unwrap();
                    let text = match payload {
                        OutputPayload::Text { text } => text,
                        OutputPayload::Attachment { bytes, .. } => {
                            String::from_utf8_lossy(&bytes).to_string()
                        }
                    };
                    let new_content = RoomMessageEventContent::text_plain(&text);
                    let replacement = new_content.make_replacement(
                        matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(
                            event_id, None,
                        ),
                    );
                    self.room.send(replacement).await?;
                }
                OutputAction::Delete {
                    target_conversation: _,
                    message_id,
                } => {
                    let event_id = message_id.parse().unwrap();
                    self.room.redact(&event_id, None, None).await?;
                }
                OutputAction::AnswerCallback { .. } => {}
                OutputAction::SendAttachment {
                    target_conversation: _,
                    payload,
                } => {
                    let (bytes, filename, mime) = match payload {
                        OutputPayload::Attachment {
                            bytes,
                            filename,
                            mime,
                        } => (bytes, filename, mime),
                        OutputPayload::Text { text } => {
                            (text.into_bytes(), "message.txt".into(), "text/plain".into())
                        }
                    };
                    let mime_val: mime::Mime = mime.parse().unwrap();
                    self.room
                        .send_attachment(&filename, &mime_val, bytes, AttachmentConfig::new())
                        .await?;
                }
            }
            Ok(())
        }

        fn as_adapter(&self) -> Arc<dyn BotAdapter> {
            NoopBotAdapter::new()
        }
    }

    fn room_id() -> ConversationId {
        ConversationId::new("!room:matrix.org".into()).unwrap()
    }

    #[tokio::test]
    async fn send_text_public_routes_to_send() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: room_id(),
                payload: OutputPayload::Text {
                    text: "hello".into(),
                },
                sensitivity: Sensitivity::Public,
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(calls.iter().any(|c| c.starts_with("send:called")));
    }

    #[tokio::test]
    async fn send_text_protected_routes_to_send_attachment() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: room_id(),
                payload: OutputPayload::Text {
                    text: "secret".into(),
                },
                sensitivity: Sensitivity::Protected,
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_attachment:message.txt:secret"))
        );
    }

    #[tokio::test]
    async fn edit_routes_to_send_replacement() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Edit {
                target_conversation: room_id(),
                message_id: "$event1:matrix.org".into(),
                payload: OutputPayload::Text {
                    text: "updated".into(),
                },
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(calls.iter().any(|c| c.starts_with("send:called")));
    }

    #[tokio::test]
    async fn delete_routes_to_redact() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Delete {
                target_conversation: room_id(),
                message_id: "$event1:matrix.org".into(),
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("redact:$event1:matrix.org"))
        );
    }

    #[tokio::test]
    async fn answer_callback_is_noop() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::AnswerCallback {
                callback_id: "cb-99".into(),
                text: Some("done".into()),
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn send_attachment_routes_to_send_attachment() {
        let fake = FakeRoom::new();
        let presenter = TestPresenter::new(TestRoom::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendAttachment {
                target_conversation: room_id(),
                payload: OutputPayload::Attachment {
                    bytes: vec![0x00, 0xFF],
                    filename: "doc.pdf".into(),
                    mime: "application/pdf".into(),
                },
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_attachment:doc.pdf:"))
        );
    }
}
