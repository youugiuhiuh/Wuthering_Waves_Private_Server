use std::sync::Arc;

use anyhow::Result;

use crate::common::{BotAdapter, MessageId, TargetId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStatus {
    NotTracked,
    Active,
    Expired,
}

pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
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

pub enum BotEvent {
    Message(MessageEvent),
    Callback(CallbackEvent),
    Command(CommandEvent),
}

impl BotEvent {
    pub fn user_id(&self) -> i64 {
        match self {
            BotEvent::Message(m) => m.user_id,
            BotEvent::Callback(c) => c.user_id.parse().unwrap_or(0),
            BotEvent::Command(c) => c.user_id,
        }
    }

    pub fn adapter(&self) -> &Arc<dyn BotAdapter> {
        match self {
            BotEvent::Message(m) => &m.adapter,
            BotEvent::Callback(c) => &c.adapter,
            BotEvent::Command(c) => &c.adapter,
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
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub reply_to_text: Option<String>,
    pub thread_root: Option<String>,
}

pub struct CommandEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub command: BotCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotCommand {
    Help,
    Start,
    Menu,
    Auth { code: String },
    SetSecurityFile,
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn message_event_constructs() {
        // MessageEvent is a plain struct — verify fields compile
        let _ = MessageEvent {
            adapter: std::sync::Arc::new(crate::common::MockBotAdapter::new()),
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
        let _ = CommandEvent {
            adapter: std::sync::Arc::new(crate::common::MockBotAdapter::new()),
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
}
