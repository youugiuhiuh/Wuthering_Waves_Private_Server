use crate::app::interaction::{
    ActorId, BusinessCommand, BusinessInput, BusinessRequest, ConversationId, Origin, PlatformId,
};

pub fn command_to_business_input(
    name: &str,
    code: Option<&str>,
    user_id: u64,
    channel_id: u64,
) -> Option<BusinessInput> {
    let origin = Origin {
        platform: PlatformId::Discord,
        actor_id: ActorId::new(user_id.to_string()).ok()?,
        conversation_id: ConversationId::new(channel_id.to_string()).ok()?,
    };
    let request = match (name, code) {
        ("help", _) => BusinessRequest::Command(BusinessCommand::Help),
        ("start", _) => BusinessRequest::Command(BusinessCommand::Start),
        ("menu", _) => BusinessRequest::Command(BusinessCommand::Menu),
        ("auth", Some(c)) => BusinessRequest::Command(BusinessCommand::Auth {
            code: c.to_string(),
        }),
        ("setsecurityfile", _) => BusinessRequest::Command(BusinessCommand::SetSecurityFile),
        _ => return None,
    };
    Some(BusinessInput { origin, request })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_to_business_input_help() {
        let input = command_to_business_input("help", None, 123, 456);
        let input = input.expect("help should map");
        assert!(matches!(
            input.request,
            BusinessRequest::Command(BusinessCommand::Help)
        ));
        assert_eq!(input.origin.platform, PlatformId::Discord);
        assert_eq!(input.origin.actor_id.as_str(), "123");
        assert_eq!(input.origin.conversation_id.as_str(), "456");
    }

    #[test]
    fn command_to_business_input_menu() {
        let input = command_to_business_input("menu", None, 99, 111);
        let input = input.expect("menu should map");
        assert!(matches!(
            input.request,
            BusinessRequest::Command(BusinessCommand::Menu)
        ));
    }

    #[test]
    fn command_to_business_input_auth() {
        let input = command_to_business_input("auth", Some("123456"), 7, 999);
        let input = input.expect("auth should map");
        match input.request {
            BusinessRequest::Command(BusinessCommand::Auth { code }) => {
                assert_eq!(code, "123456");
            }
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn command_to_business_input_setsecurityfile() {
        let input = command_to_business_input("setsecurityfile", None, 1, 2);
        let input = input.expect("setsecurityfile should map");
        assert!(matches!(
            input.request,
            BusinessRequest::Command(BusinessCommand::SetSecurityFile)
        ));
    }

    #[test]
    fn command_to_business_input_unknown_returns_none() {
        for name in ["unknown", "xray", "destruct", "ops", "warp"] {
            let result = command_to_business_input(name, None, 1, 2);
            assert!(
                result.is_none(),
                "unknown command {name} should return None"
            );
        }
    }

    #[test]
    fn command_to_business_input_auth_without_code_returns_none() {
        let result = command_to_business_input("auth", None, 1, 2);
        assert!(result.is_none(), "auth without code should return None");
    }
}
