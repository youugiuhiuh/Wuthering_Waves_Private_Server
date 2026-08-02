use crate::app::interaction::BusinessMessage;
use crate::app::output::BusinessOutput;
use anyhow::Result;
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
    async fn publish(&self, message: BusinessMessage) -> Result<()> {
        let chat_id = ChatId(message.origin.conversation_id.as_str().parse::<i64>()?);
        self.bot
            .send_message(chat_id, &message.text)
            .parse_mode(ParseMode::Html)
            .await?;
        Ok(())
    }
}
