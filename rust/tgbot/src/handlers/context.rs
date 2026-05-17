use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, MessageId};
use crate::app::state::AppState;
use anyhow::Result;

pub struct CallbackContext {
    pub bot: Bot,
    pub q: CallbackQuery,
    pub state: Arc<AppState>,
    pub chat_id: ChatId,
    pub msg_id: MessageId,
    #[allow(dead_code)]
    pub user_id: i64,
    pub data: String,
}

pub enum HandlerAction {
    Done,                     // 处理完了，直接结束
    Redirect(String),         // 内部跳转（相当于老代码的 continue）
}

pub type HandlerResult = Result<HandlerAction>;