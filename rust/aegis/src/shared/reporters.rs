use crate::common::{MessageContent, MessageId, TargetId};
use crate::core::progress::{OperationProgress, ProgressReporter};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct StatusMessageReporter {
    adapter: Arc<dyn crate::common::BotAdapter>,
    target: TargetId,
    msg_id: Mutex<Option<MessageId>>,
}

impl StatusMessageReporter {
    pub fn new(adapter: Arc<dyn crate::common::BotAdapter>, target: TargetId) -> Self {
        Self {
            adapter,
            target,
            msg_id: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ProgressReporter for StatusMessageReporter {
    async fn report(&self, progress: OperationProgress) -> anyhow::Result<()> {
        match progress {
            OperationProgress::Started(text) => {
                let msg_id = self
                    .adapter
                    .send_message(&self.target, MessageContent { text, markup: None })
                    .await?;
                *self.msg_id.lock().await = Some(msg_id);
            }
            OperationProgress::Advanced(text) => {
                let mut guard = self.msg_id.lock().await;
                match guard.as_ref() {
                    Some(msg_id) => {
                        self.adapter
                            .edit_message(
                                &self.target,
                                msg_id,
                                MessageContent { text, markup: None },
                            )
                            .await?;
                    }
                    None => {
                        let msg_id = self
                            .adapter
                            .send_message(&self.target, MessageContent { text, markup: None })
                            .await?;
                        *guard = Some(msg_id);
                    }
                }
            }
            OperationProgress::Finished(text) => {
                self.adapter
                    .send_message(&self.target, MessageContent { text, markup: None })
                    .await?;
            }
        }
        Ok(())
    }
}

pub struct SendMessageReporter {
    adapter: Arc<dyn crate::common::BotAdapter>,
    target: TargetId,
}

impl SendMessageReporter {
    pub fn new(adapter: Arc<dyn crate::common::BotAdapter>, target: TargetId) -> Self {
        Self { adapter, target }
    }
}

#[async_trait]
impl ProgressReporter for SendMessageReporter {
    async fn report(&self, progress: OperationProgress) -> anyhow::Result<()> {
        let text = match progress {
            OperationProgress::Started(text)
            | OperationProgress::Advanced(text)
            | OperationProgress::Finished(text) => text,
        };
        self.adapter
            .send_message(&self.target, MessageContent { text, markup: None })
            .await?;
        Ok(())
    }
}
