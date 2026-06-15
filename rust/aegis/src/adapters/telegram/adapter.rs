use crate::adapters::common::{BotAdapter, Markup, MessageContent, MessageId, Platform, TargetId};
use anyhow::Result;
use async_trait::async_trait;
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
