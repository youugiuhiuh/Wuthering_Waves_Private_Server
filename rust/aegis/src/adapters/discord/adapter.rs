use crate::adapters::common::{
    BotAdapter, Markup, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serenity::all::{ChannelId, MessageId as SerenityMessageId};
use serenity::http::Http;
use std::sync::Arc;

pub struct DiscordAdapter {
    http: Arc<Http>,
}

impl DiscordAdapter {
    pub fn new(http: Arc<Http>) -> Self {
        Self { http }
    }

    pub fn inner_http(&self) -> &Http {
        &self.http
    }
}

#[async_trait]
impl BotAdapter for DiscordAdapter {
    fn platform(&self) -> Platform {
        Platform::Discord
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let channel_id = ChannelId::new(target.0.parse::<u64>()?);
        let mut builder = serenity::all::CreateMessage::new().content(content.text);
        if let Some(ref markup) = content.markup {
            let components = convert_markup_discord(markup);
            builder = builder.components(components);
        }
        let msg = channel_id
            .send_message(&self.http, builder)
            .await
            .context("发送 Discord 消息失败")?;
        Ok(MessageId(msg.id.to_string()))
    }

    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let channel_id = ChannelId::new(target.0.parse::<u64>()?);
        let message_id = SerenityMessageId::new(msg_id.0.parse::<u64>()?);
        let mut builder = serenity::all::EditMessage::new().content(content.text);
        if let Some(ref markup) = content.markup {
            let components = convert_markup_discord(markup);
            builder = builder.components(components);
        }
        channel_id
            .edit_message(&self.http, message_id, builder)
            .await
            .context("编辑 Discord 消息失败")?;
        Ok(())
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let channel_id = ChannelId::new(target.0.parse::<u64>()?);
        let message_id = SerenityMessageId::new(msg_id.0.parse::<u64>()?);
        channel_id
            .delete_message(&self.http, message_id)
            .await
            .context("删除 Discord 消息失败")?;
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
        let response = reqwest::get(file_id).await?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::DISCORD
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::common::PlatformCapabilities;

    #[test]
    fn discord_capabilities_matches_expected() {
        let caps = PlatformCapabilities::DISCORD;
        assert!(caps.can_edit_message);
        assert!(caps.can_delete_message);
        assert!(!caps.has_file_transfer);
    }
}

fn convert_markup_discord(markup: &Markup) -> Vec<serenity::all::CreateActionRow> {
    markup
        .buttons
        .iter()
        .map(|row| {
            let buttons: Vec<serenity::all::CreateButton> = row
                .iter()
                .map(|btn| {
                    serenity::all::CreateButton::new(&btn.data)
                        .label(&btn.text)
                        .style(serenity::all::ButtonStyle::Primary)
                })
                .collect();
            serenity::all::CreateActionRow::Buttons(buttons)
        })
        .collect()
}
