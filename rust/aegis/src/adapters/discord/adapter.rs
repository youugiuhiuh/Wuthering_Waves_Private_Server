use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, Markup, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId, VerifiedAttachment, consume_stream,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::stream;
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

    async fn download_attachment(
        &self,
        attachment: &Attachment,
        expected_sha256: Option<[u8; 32]>,
    ) -> std::result::Result<VerifiedAttachment, AttachmentError> {
        consume_stream(attachment, expected_sha256, || async {
            let response = reqwest::get(&attachment.file_id)
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|_| AttachmentError::Transport)?;
            Ok(stream::try_unfold(response, |mut response| async move {
                response
                    .chunk()
                    .await
                    .map(|chunk| chunk.map(|bytes| (bytes, response)))
                    .map_err(|_| AttachmentError::Transport)
            }))
        })
        .await
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::DISCORD
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::{Attachment, AttachmentError, MAX_ATTACHMENT_BYTES};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn discord_capabilities_matches_expected() {
        let caps = PlatformCapabilities::DISCORD;
        assert!(caps.can_edit_message);
        assert!(caps.can_delete_message);
        assert!(!caps.has_file_transfer);
    }

    #[tokio::test]
    async fn discord_rejects_stream_at_common_limit_with_forged_size() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/security.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                0;
                (MAX_ATTACHMENT_BYTES + 1)
                    as usize
            ]))
            .mount(&server)
            .await;
        let adapter = DiscordAdapter::new(Arc::new(Http::new("TOKEN")));
        let error = adapter
            .download_attachment(
                &Attachment {
                    file_id: format!("{}/security.bin", server.uri()),
                    file_name: Some("security.bin".into()),
                    declared_size: Some(1),
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            AttachmentError::TooLarge {
                observed: MAX_ATTACHMENT_BYTES + 1,
                max: MAX_ATTACHMENT_BYTES,
            }
        );
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
