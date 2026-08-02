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
pub struct InboundAttachment {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionButton {
    pub label: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPayload {
    Text {
        text: String,
    },
    Attachment {
        bytes: Vec<u8>,
        filename: String,
        mime: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputAction {
    SendText {
        target_conversation: ConversationId,
        payload: OutputPayload,
        sensitivity: Sensitivity,
    },
    Edit {
        target_conversation: ConversationId,
        message_id: String,
        payload: OutputPayload,
    },
    Delete {
        target_conversation: ConversationId,
        message_id: String,
    },
    AnswerCallback {
        callback_id: String,
        text: Option<String>,
    },
    SendAttachment {
        target_conversation: ConversationId,
        payload: OutputPayload,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessMessage {
    pub origin: Origin,
    pub action: OutputAction,
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

    #[test]
    fn inbound_attachment_carries_bytes_and_filename() {
        let _att = InboundAttachment {
            bytes: vec![0x00, 0xFF, 0x42],
            filename: "config.json".into(),
            mime: "application/json".into(),
        };
    }

    #[test]
    fn action_button_carries_label_and_callback_data() {
        let _btn = ActionButton {
            label: "Continue".into(),
            data: "continue:yes".into(),
        };
    }

    #[test]
    fn output_payload_text_carries_content() {
        let _payload = OutputPayload::Text {
            text: "hello world".into(),
        };
    }

    #[test]
    fn output_payload_attachment_carries_bytes_and_name() {
        let _payload = OutputPayload::Attachment {
            bytes: vec![0x00, 0xFF],
            filename: "file.bin".into(),
            mime: "application/octet-stream".into(),
        };
    }

    #[test]
    fn output_action_send_text_carries_payload_and_target() {
        let _action = OutputAction::SendText {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            payload: OutputPayload::Text {
                text: "ping".into(),
            },
            sensitivity: Sensitivity::Public,
        };
    }

    #[test]
    fn output_action_edit_carries_target_msg_id_and_payload() {
        let _action = OutputAction::Edit {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            message_id: "msg-42".into(),
            payload: OutputPayload::Text {
                text: "updated".into(),
            },
        };
    }

    #[test]
    fn output_action_delete_carries_target_and_msg_id() {
        let _action = OutputAction::Delete {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            message_id: "msg-42".into(),
        };
    }

    #[test]
    fn output_action_answer_callback_carries_callback_id_and_text() {
        let _action = OutputAction::AnswerCallback {
            callback_id: "cb-99".into(),
            text: Some("done".into()),
        };
    }

    #[test]
    fn output_action_send_attachment_carries_target_and_payload() {
        let _action = OutputAction::SendAttachment {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            payload: OutputPayload::Attachment {
                bytes: vec![0x00],
                filename: "doc.pdf".into(),
                mime: "application/pdf".into(),
            },
        };
    }

    #[test]
    fn business_message_carries_output_action() {
        let _msg = BusinessMessage {
            origin: Origin {
                platform: PlatformId::Telegram,
                actor_id: ActorId::new("1".into()).unwrap(),
                conversation_id: ConversationId::new("c".into()).unwrap(),
            },
            action: OutputAction::SendText {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                payload: OutputPayload::Text {
                    text: "hello".into(),
                },
                sensitivity: Sensitivity::Public,
            },
        };
    }

    #[test]
    fn output_action_coverages_all_platform_operations() {
        let _ = [
            OutputAction::SendText {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                payload: OutputPayload::Text { text: "t".into() },
                sensitivity: Sensitivity::Public,
            },
            OutputAction::Edit {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                message_id: "m".into(),
                payload: OutputPayload::Text { text: "t".into() },
            },
            OutputAction::Delete {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                message_id: "m".into(),
            },
            OutputAction::AnswerCallback {
                callback_id: "cb".into(),
                text: Some("ok".into()),
            },
            OutputAction::SendAttachment {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                payload: OutputPayload::Attachment {
                    bytes: vec![],
                    filename: "f".into(),
                    mime: "bin".into(),
                },
            },
        ];
    }

    #[test]
    fn cross_platform_identity_isolation_via_origin() {
        let tg = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("user-1".into()).unwrap(),
            conversation_id: ConversationId::new("chat-1".into()).unwrap(),
        };
        let dc = Origin {
            platform: PlatformId::Discord,
            actor_id: ActorId::new("user-1".into()).unwrap(),
            conversation_id: ConversationId::new("chat-1".into()).unwrap(),
        };
        let mx = Origin {
            platform: PlatformId::Matrix,
            actor_id: ActorId::new("user-1".into()).unwrap(),
            conversation_id: ConversationId::new("chat-1".into()).unwrap(),
        };
        assert_ne!(tg, dc);
        assert_ne!(tg, mx);
        assert_ne!(dc, mx);
    }

    #[test]
    fn sensitivity_type_exists_and_has_public_protected_variants() {
        assert_eq!(Sensitivity::Public, Sensitivity::Public);
        assert_eq!(Sensitivity::Protected, Sensitivity::Protected);
    }

    #[test]
    fn business_message_carries_origin_for_cross_platform_identity() {
        let tg = Origin {
            platform: PlatformId::Telegram,
            actor_id: ActorId::new("1".into()).unwrap(),
            conversation_id: ConversationId::new("c".into()).unwrap(),
        };
        let msg = BusinessMessage {
            origin: tg.clone(),
            action: OutputAction::SendText {
                target_conversation: ConversationId::new("c".into()).unwrap(),
                payload: OutputPayload::Text {
                    text: "secret".into(),
                },
                sensitivity: Sensitivity::Protected,
            },
        };
        assert_eq!(msg.origin.platform, PlatformId::Telegram);
        assert_eq!(msg.origin.actor_id.as_str(), "1");
    }
}
