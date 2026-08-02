use crate::app::interaction::{OutputAction, OutputPayload, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::{Context, Result};
use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

pub struct TelegramPresenter {
    bot: Bot,
}

impl TelegramPresenter {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

#[async_trait]
impl BusinessOutput for TelegramPresenter {
    async fn publish(&self, action: OutputAction) -> Result<()> {
        match action {
            OutputAction::SendText {
                target_conversation,
                payload,
                sensitivity,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
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
                        self.bot
                            .send_document(
                                chat_id,
                                teloxide::types::InputFile::memory(text.into_bytes()),
                            )
                            .await?;
                    }
                    Sensitivity::Public => match payload {
                        OutputPayload::Text { text } => {
                            self.bot
                                .send_message(chat_id, &text)
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                        OutputPayload::Attachment {
                            bytes,
                            filename: _,
                            mime: _,
                        } => {
                            self.bot
                                .send_document(chat_id, teloxide::types::InputFile::memory(bytes))
                                .await?;
                        }
                    },
                }
            }
            OutputAction::Edit {
                target_conversation,
                message_id,
                payload,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
                let mid = message_id.parse::<i32>().context("invalid message id")?;
                let text = match payload {
                    OutputPayload::Text { text } => text,
                    OutputPayload::Attachment {
                        bytes,
                        filename: _,
                        mime: _,
                    } => String::from_utf8_lossy(&bytes).to_string(),
                };
                self.bot
                    .edit_message_text(chat_id, teloxide::types::MessageId(mid), &text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            OutputAction::Delete {
                target_conversation,
                message_id,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
                let mid = message_id.parse::<i32>().context("invalid message id")?;
                self.bot
                    .delete_message(chat_id, teloxide::types::MessageId(mid))
                    .await?;
            }
            OutputAction::AnswerCallback { callback_id, text } => {
                let mut q = self.bot.answer_callback_query(&callback_id);
                if let Some(t) = &text {
                    q = q.text(t);
                }
                q.await?;
            }
            OutputAction::SendAttachment {
                target_conversation,
                payload,
            } => {
                let chat_id = ChatId(target_conversation.as_str().parse::<i64>()?);
                match payload {
                    OutputPayload::Attachment {
                        bytes,
                        filename: _,
                        mime: _,
                    } => {
                        self.bot
                            .send_document(chat_id, teloxide::types::InputFile::memory(bytes))
                            .await?;
                    }
                    OutputPayload::Text { text } => {
                        self.bot
                            .send_document(
                                chat_id,
                                teloxide::types::InputFile::memory(text.into_bytes()),
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

    #[derive(Clone)]
    struct FakeBot {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeBot {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[derive(Clone)]
    struct TestBot {
        inner: Arc<FakeBot>,
    }

    impl TestBot {
        fn new(inner: Arc<FakeBot>) -> Self {
            Self { inner }
        }
        async fn send_message(&self, chat_id: i64, text: &str) -> Result<teloxide::types::Message> {
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("send_message:{}:{}", chat_id, text));
            Err(anyhow::anyhow!("test bot"))
        }
        async fn send_document(
            &self,
            chat_id: i64,
            data: Vec<u8>,
        ) -> Result<teloxide::types::Message> {
            let text = String::from_utf8_lossy(&data).to_string();
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("send_document:{}:{}", chat_id, text));
            Err(anyhow::anyhow!("test bot"))
        }
        async fn edit_message_text(
            &self,
            chat_id: i64,
            message_id: teloxide::types::MessageId,
            text: &str,
        ) -> Result<teloxide::types::Message> {
            self.inner.calls.lock().unwrap().push(format!(
                "edit_message_text:{}:{}:{}",
                chat_id, message_id.0, text
            ));
            Err(anyhow::anyhow!("test bot"))
        }
        async fn delete_message(
            &self,
            chat_id: i64,
            message_id: teloxide::types::MessageId,
        ) -> Result<()> {
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("delete_message:{}:{}", chat_id, message_id.0));
            Ok(())
        }
        async fn answer_callback_query(
            &self,
            callback_query_id: &str,
        ) -> Result<teloxide::types::CallbackQuery> {
            self.inner
                .calls
                .lock()
                .unwrap()
                .push(format!("answer_callback_query:{}", callback_query_id));
            Err(anyhow::anyhow!("test bot"))
        }
    }

    struct TestPresenter {
        bot: TestBot,
    }

    impl TestPresenter {
        fn new(bot: TestBot) -> Self {
            Self { bot }
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
                    let chat_id = target_conversation.as_str().parse::<i64>().unwrap();
                    match sensitivity {
                        Sensitivity::Protected => {
                            let text = match payload {
                                OutputPayload::Text { text } => text,
                                OutputPayload::Attachment { bytes, .. } => {
                                    String::from_utf8_lossy(&bytes).to_string()
                                }
                            };
                            self.bot.send_document(chat_id, text.into_bytes()).await?;
                        }
                        Sensitivity::Public => match payload {
                            OutputPayload::Text { text } => {
                                self.bot.send_message(chat_id, &text).await?;
                            }
                            OutputPayload::Attachment { bytes, .. } => {
                                self.bot.send_document(chat_id, bytes).await?;
                            }
                        },
                    }
                }
                OutputAction::Edit {
                    target_conversation,
                    message_id,
                    payload,
                } => {
                    let chat_id = target_conversation.as_str().parse::<i64>().unwrap();
                    let mid: i32 = message_id.parse().unwrap();
                    let text = match payload {
                        OutputPayload::Text { text } => text,
                        OutputPayload::Attachment { bytes, .. } => {
                            String::from_utf8_lossy(&bytes).to_string()
                        }
                    };
                    self.bot
                        .edit_message_text(chat_id, teloxide::types::MessageId(mid), &text)
                        .await?;
                }
                OutputAction::Delete {
                    target_conversation,
                    message_id,
                } => {
                    let chat_id = target_conversation.as_str().parse::<i64>().unwrap();
                    let mid: i32 = message_id.parse().unwrap();
                    self.bot
                        .delete_message(chat_id, teloxide::types::MessageId(mid))
                        .await?;
                }
                OutputAction::AnswerCallback { callback_id, text } => {
                    self.bot.answer_callback_query(&callback_id).await?;
                    let _ = text;
                }
                OutputAction::SendAttachment {
                    target_conversation,
                    payload,
                } => {
                    let chat_id = target_conversation.as_str().parse::<i64>().unwrap();
                    match payload {
                        OutputPayload::Attachment { bytes, .. } => {
                            self.bot.send_document(chat_id, bytes).await?;
                        }
                        OutputPayload::Text { text } => {
                            self.bot.send_document(chat_id, text.into_bytes()).await?;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    fn chat_id() -> ConversationId {
        ConversationId::new("123456".into()).unwrap()
    }

    #[tokio::test]
    async fn send_text_public_routes_to_send_message() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: chat_id(),
                payload: OutputPayload::Text {
                    text: "hello".into(),
                },
                sensitivity: Sensitivity::Public,
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("send_message:123456:hello"))
        );
    }

    #[tokio::test]
    async fn send_text_protected_routes_to_send_document_as_attachment() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendText {
                target_conversation: chat_id(),
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
                .any(|c| c.starts_with("send_document:123456:secret"))
        );
    }

    #[tokio::test]
    async fn edit_routes_to_edit_message_text() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Edit {
                target_conversation: chat_id(),
                message_id: "42".into(),
                payload: OutputPayload::Text {
                    text: "updated".into(),
                },
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("edit_message_text:123456:42:updated"))
        );
    }

    #[tokio::test]
    async fn delete_routes_to_delete_message() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::Delete {
                target_conversation: chat_id(),
                message_id: "42".into(),
            })
            .await
            .unwrap();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("delete_message:123456:42"))
        );
    }

    #[tokio::test]
    async fn answer_callback_routes_to_answer_callback_query() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::AnswerCallback {
                callback_id: "cb-99".into(),
                text: Some("done".into()),
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("answer_callback_query:cb-99"))
        );
    }

    #[tokio::test]
    async fn send_attachment_routes_to_send_document() {
        let fake = FakeBot::new();
        let presenter = TestPresenter::new(TestBot::new(Arc::new(fake.clone())));

        presenter
            .publish(OutputAction::SendAttachment {
                target_conversation: chat_id(),
                payload: OutputPayload::Attachment {
                    bytes: vec![0x00, 0xFF],
                    filename: "file.pdf".into(),
                    mime: "application/pdf".into(),
                },
            })
            .await
            .unwrap_err();

        let calls = fake.calls();
        assert!(calls.iter().any(|c| c.starts_with("send_document:123456:")));
    }
}
