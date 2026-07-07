use std::sync::Arc;

use crate::adapters::common::{BotAdapter, MessageId, TargetId};
use crate::app::state::AppState;

pub struct HandlerContext<'a> {
    pub adapter: &'a dyn BotAdapter,
    pub target: TargetId,
    pub state: &'a Arc<AppState>,
    pub user_id: i64,
    pub data: String,
    pub msg_id: Option<MessageId>,
}
