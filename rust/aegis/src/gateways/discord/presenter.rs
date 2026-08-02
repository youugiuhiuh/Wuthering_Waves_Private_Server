use crate::app::interaction::{BusinessMessage, Sensitivity};
use crate::app::output::BusinessOutput;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serenity::all::{ChannelId, CreateAttachment};
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
    async fn publish(&self, message: BusinessMessage) -> Result<()> {
        let channel_id = ChannelId::new(
            message
                .origin
                .conversation_id
                .as_str()
                .parse::<u64>()
                .context("invalid channel id")?,
        );
        match message.sensitivity {
            Sensitivity::Protected => {
                let msg = channel_id
                    .send_message(
                        &self.http,
                        serenity::all::CreateMessage::new().add_file(CreateAttachment::bytes(
                            message.text.into_bytes(),
                            "message.txt",
                        )),
                    )
                    .await
                    .context("发送 Discord 文件消息失败")?;
                let _ = msg;
            }
            Sensitivity::Public => {
                let msg = channel_id
                    .send_message(
                        &self.http,
                        serenity::all::CreateMessage::new().content(&message.text),
                    )
                    .await
                    .context("发送 Discord 消息失败")?;
                let _ = msg;
            }
        }
        Ok(())
    }
}
