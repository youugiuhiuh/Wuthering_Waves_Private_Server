use crate::core::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformId {
    Telegram,
    Matrix,
    Discord,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorId(String);

impl ActorId {
    pub fn new(raw: String) -> Result<Self> {
        if raw.trim().is_empty() {
            return Err(AppError::InvalidParameter(
                "actor id cannot be empty".into(),
            ));
        }
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(String);

impl ConversationId {
    pub fn new(raw: String) -> Result<Self> {
        if raw.trim().is_empty() {
            return Err(AppError::InvalidParameter(
                "conversation id cannot be empty".into(),
            ));
        }
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    pub platform: PlatformId,
    pub actor_id: ActorId,
    pub conversation_id: ConversationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusinessCommand {
    Help,
    Start,
    Menu,
    Auth { code: String },
    SetSecurityFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusinessRequest {
    Command(BusinessCommand),
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessInput {
    pub origin: Origin,
    pub request: BusinessRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusinessResult {
    Ok,
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sensitivity {
    #[default]
    Public,
    Protected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessMessage {
    pub origin: Origin,
    pub text: String,
    pub sensitivity: Sensitivity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_raw_ids_on_different_platforms_are_distinct_origins() {
        let tg = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("42".into()).unwrap(),
            conversation_id: ConversationId::new("chat".into()).unwrap(),
        };
        let mx = Origin {
            platform: PlatformId::Matrix,
            actor_id: ActorId::new("42".into()).unwrap(),
            conversation_id: ConversationId::new("chat".into()).unwrap(),
        };
        assert_ne!(tg, mx);
    }

    #[test]
    fn empty_ids_are_rejected() {
        assert!(matches!(
            ActorId::new(String::new()),
            Err(crate::core::error::AppError::InvalidParameter(_))
        ));
        assert!(matches!(
            ActorId::new("   ".into()),
            Err(crate::core::error::AppError::InvalidParameter(_))
        ));
        assert!(matches!(
            ConversationId::new(String::new()),
            Err(crate::core::error::AppError::InvalidParameter(_))
        ));
        assert!(matches!(
            ConversationId::new("\t".into()),
            Err(crate::core::error::AppError::InvalidParameter(_))
        ));
    }

    #[test]
    fn ids_expose_their_raw_value() {
        let actor = ActorId::new("u-1".into()).unwrap();
        let conversation = ConversationId::new("c-1".into()).unwrap();
        assert_eq!(actor.as_str(), "u-1");
        assert_eq!(conversation.as_str(), "c-1");
    }

    #[test]
    fn business_command_covers_the_single_vocabulary() {
        let cmds = [
            BusinessCommand::Help,
            BusinessCommand::Start,
            BusinessCommand::Menu,
            BusinessCommand::Auth {
                code: "123456".into(),
            },
            BusinessCommand::SetSecurityFile,
        ];
        assert_eq!(cmds.len(), 5);
    }
}
