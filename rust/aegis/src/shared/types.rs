use std::sync::Arc;

use anyhow::Result;

use crate::app::interaction::Origin;
use crate::app::output::BusinessOutput;
use crate::common::{MessageId, TargetId};
use crate::core::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStatus {
    NotTracked,
    Active,
    Expired,
}

pub struct CallbackEvent {
    pub output: Arc<dyn BusinessOutput>,
    pub origin: Origin,
    pub target: TargetId,
    pub user_id: String,
    pub msg_id: MessageId,
    pub data: String,
    pub callback_id: String,
    pub session_timeout_secs: u64,
}

pub enum HandlerAction {
    Done,
    Redirect(String),
}

pub type HandlerResult = Result<HandlerAction>;
pub type DispatchResult = Result<Option<HandlerAction>>;

/// Legacy ingress for current native gateways. Tasks 10-12 migrate each native
/// SDK to construct `BusinessRequest` directly; this type is removed in Task 14.
pub enum BotEvent {
    Message(MessageEvent),
    Callback(CallbackEvent),
    Command(CommandEvent),
}

impl BotEvent {
    pub fn user_id(&self) -> crate::core::error::Result<i64> {
        match self {
            BotEvent::Message(m) => Ok(m.user_id),
            BotEvent::Callback(c) => c
                .user_id
                .parse()
                .map_err(|_| AppError::InvalidParameter("callback user id is not numeric".into())),
            BotEvent::Command(c) => Ok(c.user_id),
        }
    }

    pub fn adapter(&self) -> &Arc<dyn BusinessOutput> {
        match self {
            BotEvent::Message(m) => &m.output,
            BotEvent::Callback(c) => &c.output,
            BotEvent::Command(c) => &c.output,
        }
    }

    pub fn target(&self) -> &TargetId {
        match self {
            BotEvent::Message(m) => &m.target,
            BotEvent::Callback(c) => &c.target,
            BotEvent::Command(c) => &c.target,
        }
    }
}

pub struct MessageEvent {
    pub output: Arc<dyn BusinessOutput>,
    pub origin: Origin,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub reply_to_text: Option<String>,
    pub thread_root: Option<String>,
}

pub struct CommandEvent {
    pub output: Arc<dyn BusinessOutput>,
    pub origin: Origin,
    pub target: TargetId,
    pub user_id: i64,
    pub command: BotCommand,
}

pub use crate::app::interaction::BusinessCommand as BotCommand;

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn message_event_constructs() {
        use crate::app::interaction::{ActorId, ConversationId, PlatformId};
        use crate::common::BotAdapter;
        use crate::shared::commands::AdapterOutput;
        let adapter: Arc<dyn BotAdapter> = Arc::new(crate::common::MockBotAdapter::new());
        let output: Arc<dyn BusinessOutput> =
            Arc::new(AdapterOutput::new(adapter, TargetId("123".into())));
        let origin = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("42".into()).unwrap(),
            conversation_id: ConversationId::new("123".into()).unwrap(),
        };
        let _ = MessageEvent {
            output,
            origin,
            target: TargetId("123".into()),
            user_id: 42,
            text: Some("hello".into()),
            file_id: None,
            file_name: None,
            reply_to_text: None,
            thread_root: None,
        };
    }

    #[test]
    fn command_event_constructs() {
        use crate::app::interaction::{ActorId, ConversationId, PlatformId};
        use crate::common::BotAdapter;
        use crate::shared::commands::AdapterOutput;
        let adapter: Arc<dyn BotAdapter> = Arc::new(crate::common::MockBotAdapter::new());
        let output: Arc<dyn BusinessOutput> =
            Arc::new(AdapterOutput::new(adapter, TargetId("123".into())));
        let origin = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("42".into()).unwrap(),
            conversation_id: ConversationId::new("123".into()).unwrap(),
        };
        let _ = CommandEvent {
            output,
            origin,
            target: TargetId("123".into()),
            user_id: 42,
            command: BotCommand::Help,
        };
    }

    #[test]
    fn bot_command_auth_carries_code() {
        let cmd = BotCommand::Auth {
            code: "123456".into(),
        };
        assert!(matches!(cmd, BotCommand::Auth { ref code } if code == "123456"));
    }

    #[test]
    fn malformed_callback_user_id_returns_error() {
        use crate::app::interaction::{ActorId, ConversationId, PlatformId};
        use crate::common::BotAdapter;
        use crate::shared::commands::AdapterOutput;
        let adapter: Arc<dyn BotAdapter> = Arc::new(crate::common::MockBotAdapter::new());
        let output: Arc<dyn BusinessOutput> =
            Arc::new(AdapterOutput::new(adapter, TargetId("123".into())));
        let origin = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("not-a-number".into()).unwrap(),
            conversation_id: ConversationId::new("123".into()).unwrap(),
        };
        let event = BotEvent::Callback(CallbackEvent {
            output,
            origin,
            target: TargetId("123".into()),
            user_id: "not-a-number".into(),
            msg_id: MessageId("1".into()),
            data: String::new(),
            callback_id: String::new(),
            session_timeout_secs: 600,
        });
        assert!(matches!(
            event.user_id(),
            Err(AppError::InvalidParameter(_))
        ));
    }
}
