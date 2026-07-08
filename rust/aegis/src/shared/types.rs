use std::sync::Arc;

use anyhow::Result;

use crate::adapters::common::{BotAdapter, MessageId, TargetId};

pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,
    pub msg_id: MessageId,
    pub data: String,
    pub callback_id: String,
}

pub enum HandlerAction {
    Done,
    Redirect(String),
}

pub type HandlerResult = Result<HandlerAction>;
pub type DispatchResult = Result<Option<HandlerAction>>;
