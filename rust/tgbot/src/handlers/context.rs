use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, MessageId};
use crate::app::state::AppState;
use anyhow::Result;

// 封装处理一个 Callback 所需的所有上下文
pub struct CallbackContext {
    pub bot: Bot,
    pub q: CallbackQuery,
    pub state: Arc<AppState>,
    pub chat_id: ChatId,
    pub msg_id: MessageId,
    pub user_id: i64,
    pub data: String, // 当前的 callback data
}

// 核心机制：用枚举替代 loop { continue }
pub enum HandlerAction {
    Done,
    Redirect(String), // 携带新的 callback data 触发重定向
}

// 统一的返回类型
pub type HandlerResult = Result<HandlerAction>;