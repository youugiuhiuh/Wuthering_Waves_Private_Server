use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serenity::all::{
    ChannelId, CreateAttachment, CreateMessage, EditMessage, MessageId as SerenityMessageId,
};
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
                        channel_id
                            .send_message(
                                &self.http,
                                CreateMessage::new().add_file(CreateAttachment::bytes(
                                    text.into_bytes(),
                                    "message.txt",
                                )),
                            )
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
                        channel_id
                            .send_message(&self.http, CreateMessage::new().content(&text))
                            .await?;
                    }
                }
            }
            OutputAction::Edit {
                target_conversation,
                message_id,
                payload,
            } => {
                let channel_id = ChannelId::new(
                    target_conversation
                        .as_str()
                        .parse::<u64>()
                        .context("invalid channel id")?,
                );
                let msg_id = SerenityMessageId::new(
                    message_id.parse::<u64>().context("invalid message id")?,
                );
                let text = match payload {
                    OutputPayload::Text { text } => text,
                    OutputPayload::Attachment {
                        bytes,
                        filename: _,
                        mime: _,
                    } => String::from_utf8_lossy(&bytes).to_string(),
                };
                channel_id
                    .edit_message(&self.http, msg_id, EditMessage::new().content(&text))
                    .await?;
            }
            OutputAction::Delete {
                target_conversation,
                message_id,
            } => {
                let channel_id = ChannelId::new(
                    target_conversation
                        .as_str()
                        .parse::<u64>()
                        .context("invalid channel id")?,
                );
                let msg_id = SerenityMessageId::new(
                    message_id.parse::<u64>().context("invalid message id")?,
                );
                channel_id.delete_message(&self.http, msg_id).await?;
            }
            OutputAction::AnswerCallback { .. } => {}
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
                match payload {
                    OutputPayload::Attachment {
                        bytes,
                        filename,
                        mime: _,
                    } => {
                        channel_id
                            .send_message(
                                &self.http,
                                CreateMessage::new()
                                    .add_file(CreateAttachment::bytes(bytes, &filename)),
                            )
                            .await?;
                    }
                    OutputPayload::Text { text } => {
                        channel_id
                            .send_message(
                                &self.http,
                                CreateMessage::new().add_file(CreateAttachment::bytes(
                                    text.into_bytes(),
                                    "message.txt",
                                )),
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interaction::ConversationId;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    pub struct FakeHttp {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeHttp {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    struct TestHttp {
        inner: Arc<FakeHttp>,
    }

    impl TestHttp {
        fn new(inner: Arc<FakeHttp>) -> Self {
            Self { inner }
        }
    }

    struct TestPresenter {
        http: TestHttp,
    }

    impl TestPresenter {
        fn new(http: TestHttp) -> Self {
            Self { http }
        }
    }

    #[async_trait]
    impl BusinessOutput for TestPresenter {
        async fn publish(&self, action: OutputAction) -> Result<()> {
            match action {
                OutputAction::SendText {
                    target_conversation,
                    payload,
                    sensitivity,
                } => {
                    let channel_id: u64 = target_conversation.as_str().parse().unwrap();
                    match sensitivity {
                        Sensitivity::Protected => {
                            let text = match payload {
                                OutputPayload::Text { text } => text,
                                OutputPayload::Attachment { bytes, .. } => {
                                    String::from_utf8_lossy(&bytes).to_string()
                                }
                            };
                            self.http
                                .inner
                                .calls
                                .lock()
                                .unwrap()
                                .push(format!("send_message:{}:protected:{}", channel_id, text));
                        }
                        Sensitivity::Public => {
                            let text = match payload {
                                OutputPayload::Text { text } => text,
                                OutputPayload::Attachment { bytes, .. } => {
                                    String::from_utf8_lossy(&bytes).to_string()
                                }
                            };
                            self.http
                                .inner
                                .calls
                                .lock()
                                .unwrap()
                                .push(format!("send_message:{}:public:{}", channel_id, text));
                        }
                    }
                }
                OutputAction::Edit {
                    target_conversation,
                    message_id,
                    payload,
                } => {
                    let channel_id: u64 = target_conversation.as_str().parse().unwrap();
                    let text = match payload {
                        OutputPayload::Text { text } => text,
                        OutputPayload::Attachment { bytes, .. } => {
                            String::from_utf8_lossy(&bytes).to_string()
                        }
                    };
                    self.http.inner.calls.lock().unwrap().push(format!(
                        "edit_message:{}:{}:{}",
                        channel_id, message_id, text
                    ));
                }
                OutputAction::Delete {
                    target_conversation,
                    message_id,
                } => {
                    let channel_id: u64 = target_conversation.as_str().parse().unwrap();
                    self.http
                        .inner
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("delete_message:{}:{}", channel_id, message_id));
                }
                OutputAction::AnswerCallback { .. } => {}
                OutputAction::SendAttachment {
                    target_conversation,
                    payload,
                } => {
                    let channel_id: u64 = target_conversation.as_str().parse().unwrap();
                    let filename = match payload {
                        OutputPayload::Attachment { filename, .. } => filename,
                        OutputPayload::Text { .. } => "message.txt".into(),
                    };
                    self.http
                        .inner
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("send_attachment:{}:{}", channel_id, filename));
                }
            }
            Ok(())
        }
    }

    fn channel_id() -> ConversationId {
        ConversationId::new("9876543210".into()).unwrap()
    }

    #[tokio::test]
    async fn send_text_public_routes_to_send_message() {
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: channel_id(),
                payload: OutputPayload::Text {
                    text: "hello".into(),
                },
                sensitivity: Sensitivity::Public,
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_message:9876543210:public:hello"))
        );
    }

    #[tokio::test]
    async fn send_text_protected_routes_to_send_message_with_attachment() {
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: channel_id(),
                payload: OutputPayload::Text {
                    text: "secret".into(),
                },
                sensitivity: Sensitivity::Protected,
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_message:9876543210:protected:secret"))
        );
    }

    #[tokio::test]
    async fn edit_routes_to_edit_message() {
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Edit {
                target_conversation: channel_id(),
                message_id: "111222333".into(),
                payload: OutputPayload::Text {
                    text: "updated".into(),
                },
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("edit_message:9876543210:111222333:updated"))
        );
    }

    #[tokio::test]
    async fn delete_routes_to_delete_message() {
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Delete {
                target_conversation: channel_id(),
                message_id: "111222333".into(),
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("delete_message:9876543210:111222333"))
        );
    }

    #[tokio::test]
    async fn answer_callback_is_noop() {
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

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
        let fake = FakeHttp::new();
        let presenter = TestPresenter::new(TestHttp::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendAttachment {
                target_conversation: channel_id(),
                payload: OutputPayload::Attachment {
                    bytes: vec![0x00, 0xFF],
                    filename: "doc.pdf".into(),
                    mime: "application/pdf".into(),
                },
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_attachment:9876543210:doc.pdf"))
        );
    }
}
