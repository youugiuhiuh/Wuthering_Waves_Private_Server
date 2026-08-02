use crate::app::interaction::{
    ActorId, BusinessCommand, BusinessInput, BusinessRequest, ConversationId, Origin, PlatformId,
};
use crate::core::error::{AppError, Result};
use teloxide::types::{CallbackQuery, Message};

pub fn message_origin(msg: &Message) -> Result<Origin> {
    let user = msg
        .from
        .as_ref()
        .ok_or_else(|| AppError::InvalidParameter("telegram message is missing a sender".into()))?;
    Ok(Origin {
        platform: PlatformId::Telegram,
        actor_id: ActorId::new(user.id.0.to_string())?,
        conversation_id: ConversationId::new(msg.chat.id.0.to_string())?,
    })
}

pub fn command_input(msg: &Message, command: BusinessCommand) -> Result<BusinessInput> {
    Ok(BusinessInput {
        origin: message_origin(msg)?,
        request: BusinessRequest::Command(command),
    })
}

pub fn text_input(msg: &Message, text: String) -> Result<BusinessInput> {
    Ok(BusinessInput {
        origin: message_origin(msg)?,
        request: BusinessRequest::Text { text },
    })
}

pub fn callback_origin(q: &CallbackQuery) -> Result<Origin> {
    let chat_id = q
        .message
        .as_ref()
        .and_then(|m| {
            let json = serde_json::to_value(m).ok()?;
            json.get("chat")?.get("id")?.as_i64()
        })
        .ok_or_else(|| {
            AppError::InvalidParameter("callback query has no accessible message".into())
        })?;
    Ok(Origin {
        platform: PlatformId::Telegram,
        actor_id: ActorId::new(q.from.id.0.to_string())?,
        conversation_id: ConversationId::new(chat_id.to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interaction::{BusinessRequest, PlatformId};
    use crate::core::error::AppError;
    use teloxide::types::Message;

    fn make_message(has_from: bool) -> Message {
        let from = if has_from {
            r#","from":{"id":123,"is_bot":false,"first_name":"T","username":"t"}"#
        } else {
            ""
        };
        let json = format!(
            r#"{{"message_id":1,"date":0,"chat":{{"id":42,"type":"private"}}{from},"text":"hello"}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn message_origin_maps_platform_and_ids() {
        let origin = message_origin(&make_message(true)).unwrap();
        assert_eq!(origin.platform, PlatformId::Telegram);
        assert_eq!(origin.actor_id.as_str(), "123");
        assert_eq!(origin.conversation_id.as_str(), "42");
    }

    #[test]
    fn message_origin_rejects_missing_sender() {
        let err = message_origin(&make_message(false)).unwrap_err();
        assert!(matches!(err, AppError::InvalidParameter(_)));
    }

    #[test]
    fn command_input_carries_command_and_origin() {
        let input = command_input(
            &make_message(true),
            crate::app::interaction::BusinessCommand::Help,
        )
        .unwrap();
        assert!(matches!(
            input.request,
            BusinessRequest::Command(crate::app::interaction::BusinessCommand::Help)
        ));
        assert_eq!(input.origin.actor_id.as_str(), "123");
    }

    #[test]
    fn text_input_carries_text() {
        let input = text_input(&make_message(true), "hello".to_string()).unwrap();
        assert!(matches!(input.request, BusinessRequest::Text { ref text } if text == "hello"));
    }

    #[test]
    fn callback_origin_maps_from_and_chat() {
        let q: teloxide::types::CallbackQuery = serde_json::from_str(
            r#"{"id":"cb1","from":{"id":123,"is_bot":false,"first_name":"T"},"chat_instance":"ci","message":{"message_id":1,"date":0,"chat":{"id":42,"type":"private"},"text":"x"}}"#,
        )
        .unwrap();
        let origin = callback_origin(&q).unwrap();
        assert_eq!(origin.platform, PlatformId::Telegram);
        assert_eq!(origin.actor_id.as_str(), "123");
        assert_eq!(origin.conversation_id.as_str(), "42");
    }

    #[test]
    fn callback_origin_rejects_missing_message() {
        let q: teloxide::types::CallbackQuery = serde_json::from_str(
            r#"{"id":"cb1","from":{"id":123,"is_bot":false,"first_name":"T"},"chat_instance":"ci"}"#,
        )
        .unwrap();
        let err = callback_origin(&q).unwrap_err();
        assert!(matches!(err, AppError::InvalidParameter(_)));
    }
}
