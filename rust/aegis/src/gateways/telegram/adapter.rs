use crate::common::{
    BotAdapter, Markup, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use crate::core::i18n;
use crate::core::i18n::Lang;
use crate::core::system::operations::Operations;
use crate::core::system::scheduler::{ScheduledTask, TaskType, get_manager};
use anyhow::Result;
use async_trait::async_trait;
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
        let mut answer = self
            .bot
            .answer_callback_query(teloxide::types::CallbackQueryId(callback_id.to_owned()));
        if let Some(ref t) = text {
            answer = answer.text(t);
        }
        answer.await?;
        Ok(())
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        let file = self
            .bot
            .get_file(teloxide::types::FileId(file_id.to_owned()))
            .await?;
        let mut buf = Vec::with_capacity(file.size as usize);
        self.bot.download_file(&file.path, &mut buf).await?;
        Ok(buf)
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

    async fn send_typing(&self, _target: &TargetId, _active: bool) -> Result<()> {
        Ok(())
    }

    async fn send_reaction(
        &self,
        _target: &TargetId,
        _msg_id: &MessageId,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn send_message_threaded(
        &self,
        target: &TargetId,
        content: MessageContent,
        _thread_root: &str,
    ) -> Result<MessageId> {
        self.send_message(target, content).await
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
