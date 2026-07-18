use std::sync::Arc;

use anyhow::Result;

use crate::adapters::common::{BotAdapter, MessageId, Principal, TargetId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStatus {
    NotTracked,
    Active,
    Expired,
}

pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub principal: Principal,
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
    pub fn principal(&self) -> &Principal {
        match self {
            BotEvent::Message(m) => &m.principal,
            BotEvent::Callback(c) => &c.principal,
            BotEvent::Command(c) => &c.principal,
        }
    }

    pub fn user_id(&self) -> i64 {
        // ponytail: transitional, remove after all callers use principal()
        match self {
            BotEvent::Message(m) => m.principal.subject.parse().unwrap_or(0),
            BotEvent::Callback(c) => c.principal.subject.parse().unwrap_or(0),
            BotEvent::Command(c) => c.principal.subject.parse().unwrap_or(0),
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
    pub principal: Principal,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub reply_to_text: Option<String>,
}

pub struct CommandEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub principal: Principal,
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
mod principal_tests {
    use crate::adapters::common::Platform;

    use super::Principal;

    #[test]
    fn test_principals_include_platform_namespace() {
        assert_ne!(Principal::telegram(42), Principal::discord(42));
        assert_ne!(
            Principal::matrix("@admin:a.example").unwrap(),
            Principal::matrix("@admin:b.example").unwrap()
        );
    }

    #[test]
    fn test_principal_rejects_noncanonical_subjects() {
        assert!(Principal::new(Platform::Telegram, "0042").is_err());
        assert!(Principal::new(Platform::Discord, " 42").is_err());
        assert!(Principal::matrix("admin").is_err());
    }

    #[test]
    fn test_principal_accepts_valid_subjects() {
        assert!(Principal::new(Platform::Telegram, "42").is_ok());
        assert!(Principal::new(Platform::Discord, "1234567890").is_ok());
        assert!(Principal::matrix("@user:matrix.org").is_ok());
    }

    #[test]
    fn test_principal_telegram_discord_no_overflow_mix() {
        let tg = Principal::telegram(0);
        let dc = Principal::discord(0);
        assert_ne!(tg, dc);
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn message_event_constructs() {
        let _ = MessageEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            principal: Principal::telegram(42),
            text: Some("hello".into()),
            file_id: None,
            file_name: None,
            reply_to_text: None,
        };
    }

    #[test]
    fn command_event_constructs() {
        let _ = CommandEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            principal: Principal::telegram(42),
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
