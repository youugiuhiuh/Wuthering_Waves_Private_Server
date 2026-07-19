use crate::adapters::common::{
    Attachment, AttachmentError, BotAdapter, MAX_ATTACHMENT_BYTES, Markup, MessageContent,
    MessageId, Platform, PlatformCapabilities, TargetId, VerifiedAttachment, consume_stream,
};
use crate::core::i18n;
use crate::core::i18n::Lang;
use crate::core::system::operations::Operations;
use crate::core::system::scheduler::{ScheduledTask, TaskType, get_manager};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub struct TelegramAdapter {
    bot: Bot,
}

impl TelegramAdapter {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }

    pub fn inner_bot(&self) -> &Bot {
        &self.bot
    }
}

#[async_trait]
impl BotAdapter for TelegramAdapter {
    fn platform(&self) -> Platform {
        Platform::Telegram
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let chat_id = ChatId(target.0.parse::<i64>()?);
        let mut msg = self
            .bot
            .send_message(chat_id, &content.text)
            .parse_mode(ParseMode::Html);
        if let Some(ref markup) = content.markup {
            let kb = convert_markup(markup);
            msg = msg.reply_markup(kb);
        }
        let sent = msg.await?;
        Ok(MessageId(sent.id.0.to_string()))
    }

    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let chat_id = ChatId(target.0.parse::<i64>()?);
        let teloxide_id = teloxide::types::MessageId(msg_id.0.parse::<i32>()?);
        let mut msg = self
            .bot
            .edit_message_text(chat_id, teloxide_id, &content.text)
            .parse_mode(ParseMode::Html);
        if let Some(ref markup) = content.markup {
            let kb = convert_markup(markup);
            msg = msg.reply_markup(kb);
        }
        msg.await?;
        Ok(())
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let chat_id = ChatId(target.0.parse::<i64>()?);
        let teloxide_id = teloxide::types::MessageId(msg_id.0.parse::<i32>()?);
        self.bot.delete_message(chat_id, teloxide_id).await?;
        Ok(())
    }

    async fn answer_callback(
        &self,
        _target: &TargetId,
        callback_id: &str,
        text: Option<String>,
    ) -> Result<()> {
        let mut answer = self.bot.answer_callback_query(callback_id);
        if let Some(ref t) = text {
            answer = answer.text(t);
        }
        answer.await?;
        Ok(())
    }

    async fn download_attachment(
        &self,
        attachment: &Attachment,
        expected_sha256: Option<[u8; 32]>,
    ) -> std::result::Result<VerifiedAttachment, AttachmentError> {
        consume_stream(attachment, expected_sha256, || async {
            let file = self
                .bot
                .get_file(&attachment.file_id)
                .await
                .map_err(|_| AttachmentError::Transport)?;
            if file.size != u32::MAX && u64::from(file.size) > MAX_ATTACHMENT_BYTES {
                return Err(AttachmentError::MetadataTooLarge);
            }
            if let (Some(declared), size) = (attachment.declared_size, file.size)
                && size != u32::MAX
                && declared != u64::from(size)
            {
                return Err(AttachmentError::MetadataMismatch);
            }
            Ok(self
                .bot
                .download_file_stream(&file.path)
                .map(|chunk| chunk.map_err(|_| AttachmentError::Transport)))
        })
        .await
    }

    async fn set_system_locale(&self, lang: Lang) -> Result<()> {
        let tz = i18n::lang_to_timezone(lang);

        match tokio::process::Command::new("timedatectl")
            .args(["set-timezone", tz])
            .output()
            .await
        {
            Ok(o) if !o.status.success() => {
                log::warn!("设置系统时区 {} 失败: exit {:?}", tz, o.status.code());
            }
            Err(e) => log::warn!("设置系统时区 {} 失败: {}", tz, e),
            _ => {}
        }

        if let Err(e) = Operations::set_apt_daily_timer().await {
            log::warn!("覆盖 apt-daily timer 失败: {}", e);
        }

        if let Err(e) =
            Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME).await
        {
            log::warn!("安全更新初始化失败: {}", e);
        }

        if let Some(manager) = get_manager().await {
            let geo_task = ScheduledTask::new_with_timezone(TaskType::GeoUpdate, "0 1 * * 1", tz);
            let _ = manager.add_new_task(geo_task).await;
        }

        Ok(())
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::TELEGRAM
    }
}

fn convert_markup(markup: &Markup) -> InlineKeyboardMarkup {
    let rows: Vec<Vec<InlineKeyboardButton>> = markup
        .buttons
        .iter()
        .map(|row| {
            row.iter()
                .map(|btn| InlineKeyboardButton::callback(&btn.text, &btn.data))
                .collect()
        })
        .collect();
    InlineKeyboardMarkup::new(rows)
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use crate::adapters::common::{Attachment, AttachmentError, MAX_ATTACHMENT_BYTES};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn telegram_rejects_stream_at_common_limit_without_declared_size() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botTOKEN/GetFile"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "file_id": "opaque",
                    "file_unique_id": "unique",
                    "file_size": MAX_ATTACHMENT_BYTES,
                    "file_path": "security.bin"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/file/botTOKEN/security.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                0;
                (MAX_ATTACHMENT_BYTES + 1)
                    as usize
            ]))
            .mount(&server)
            .await;

        let bot = Bot::new("TOKEN").set_api_url(server.uri().parse().unwrap());
        let adapter = TelegramAdapter::new(bot);
        let error = adapter
            .download_attachment(
                &Attachment {
                    file_id: "opaque".into(),
                    file_name: Some("security.bin".into()),
                    declared_size: None,
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
