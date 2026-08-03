use crate::app::interaction::{
    BusinessCommand, BusinessInput, BusinessRequest, BusinessResult, OutputAction, OutputPayload,
    Sensitivity,
};
use crate::app::output::BusinessOutput;
use crate::app::state::AppState;
use crate::core::error::{AppError, Result};
use std::time::Instant;

pub struct ApplicationService;

impl ApplicationService {
    pub async fn handle(
        &self,
        input: &BusinessInput,
        state: &AppState,
        output: &dyn BusinessOutput,
    ) -> Result<BusinessResult> {
        let BusinessRequest::Command(command) = &input.request else {
            return Ok(BusinessResult::Ok);
        };
        match command {
            BusinessCommand::Help => {
                let text = rust_i18n::t!("help.text").into_owned();
                output
                    .publish(OutputAction::SendText {
                        target_conversation: input.origin.conversation_id.clone(),
                        payload: OutputPayload::Text { text: text.clone() },
                        sensitivity: Sensitivity::default(),
                    })
                    .await
                    .map_err(to_app_error)?;
                Ok(BusinessResult::Message(text))
            }
            BusinessCommand::Start => {
                let text = format!(
                    "{}\n\n{}",
                    rust_i18n::t!("welcome.title"),
                    rust_i18n::t!("welcome.prompt")
                );
                output
                    .publish(OutputAction::SendText {
                        target_conversation: input.origin.conversation_id.clone(),
                        payload: OutputPayload::Text { text: text.clone() },
                        sensitivity: Sensitivity::default(),
                    })
                    .await
                    .map_err(to_app_error)?;
                Ok(BusinessResult::Message(text))
            }
            BusinessCommand::Menu => {
                let user_id = parse_actor_id(input)?;
                if !state.is_authorized(user_id).await {
                    let text = rust_i18n::t!("auth.required").into_owned();
                    output
                        .publish(OutputAction::SendText {
                            target_conversation: input.origin.conversation_id.clone(),
                            payload: OutputPayload::Text { text: text.clone() },
                            sensitivity: Sensitivity::default(),
                        })
                        .await
                        .map_err(to_app_error)?;
                    return Ok(BusinessResult::Message(text));
                }
                Ok(BusinessResult::Ok)
            }
            BusinessCommand::SetSecurityFile => {
                let user_id = parse_actor_id(input)?;
                if !state.is_recently_authenticated(user_id).await {
                    let text = rust_i18n::t!("auth.recent_auth_required").into_owned();
                    output
                        .publish(OutputAction::SendText {
                            target_conversation: input.origin.conversation_id.clone(),
                            payload: OutputPayload::Text { text: text.clone() },
                            sensitivity: Sensitivity::default(),
                        })
                        .await
                        .map_err(to_app_error)?;
                    return Ok(BusinessResult::Message(text));
                }
                let text = rust_i18n::t!("bot_commands.security_file_prompt").into_owned();
                output
                    .publish(OutputAction::SendText {
                        target_conversation: input.origin.conversation_id.clone(),
                        payload: OutputPayload::Text { text: text.clone() },
                        sensitivity: Sensitivity::default(),
                    })
                    .await
                    .map_err(to_app_error)?;
                state
                    .start_security_file_input(
                        input.origin.conversation_id.as_str().to_string(),
                        Instant::now(),
                    )
                    .await;
                Ok(BusinessResult::Message(text))
            }
            BusinessCommand::Auth { .. } => Ok(BusinessResult::Ok),
        }
    }
}

fn parse_actor_id(input: &BusinessInput) -> Result<i64> {
    input
        .origin
        .actor_id
        .as_str()
        .parse::<i64>()
        .map_err(|_| AppError::InvalidParameter("actor id is not a numeric user id".into()))
}

fn to_app_error(err: anyhow::Error) -> AppError {
    AppError::Service(err.to_string())
}

#[cfg(test)]
mod tests {
    use crate::app::interaction::{
        ActorId, BusinessCommand, BusinessInput, BusinessRequest, BusinessResult, ConversationId,
        Origin, OutputAction, OutputPayload, PlatformId,
    };
    use crate::app::output::{BusinessOutput, NoopBotAdapter};
    use crate::app::service::ApplicationService;
    use crate::app::state::AppState;
    use crate::common::BotAdapter;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use anyhow::Result as AnyResult;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, AnyResult<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct RecordingOutput {
        published: Mutex<Vec<OutputAction>>,
    }

    impl RecordingOutput {
        fn new() -> Self {
            Self {
                published: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl BusinessOutput for RecordingOutput {
        async fn publish(&self, action: OutputAction) -> AnyResult<()> {
            self.published.lock().unwrap().push(action);
            Ok(())
        }

        fn as_adapter(&self) -> Arc<dyn BotAdapter> {
            NoopBotAdapter::new()
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            Some(42),
            None,
            Some(
                TotpManager::new(&secrecy::SecretString::from(
                    TotpManager::generate_new_secret(),
                ))
                .unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
        )
    }

    fn origin(platform: PlatformId) -> Origin {
        Origin {
            platform,
            actor_id: ActorId::new("42".into()).unwrap(),
            conversation_id: ConversationId::new("chat".into()).unwrap(),
        }
    }

    fn command_input(platform: PlatformId, command: BusinessCommand) -> BusinessInput {
        BusinessInput {
            origin: origin(platform),
            request: BusinessRequest::Command(command),
        }
    }

    fn extract_text(action: &OutputAction) -> Option<&str> {
        match action {
            OutputAction::SendText { payload, .. } => match payload {
                OutputPayload::Text { text } => Some(text),
                OutputPayload::Attachment { .. } => None,
            },
            _ => None,
        }
    }

    #[tokio::test]
    async fn help_is_identical_across_platforms() {
        let service = ApplicationService;
        for platform in [
            PlatformId::Telegram,
            PlatformId::Matrix,
            PlatformId::Discord,
        ] {
            let output = RecordingOutput::new();
            let state = make_state();
            let result = service
                .handle(
                    &command_input(platform, BusinessCommand::Help),
                    &state,
                    &output,
                )
                .await
                .unwrap();
            let expected = rust_i18n::t!("help.text").into_owned();
            assert_eq!(result, BusinessResult::Message(expected.clone()));
            let published = output.published.lock().unwrap();
            assert_eq!(published.len(), 1);
            assert_eq!(extract_text(&published[0]), Some(expected.as_str()));
        }
    }

    #[tokio::test]
    async fn menu_denied_when_not_authorized() {
        let service = ApplicationService;
        let output = RecordingOutput::new();
        let state = make_state();
        let result = service
            .handle(
                &command_input(PlatformId::Telegram, BusinessCommand::Menu),
                &state,
                &output,
            )
            .await
            .unwrap();
        let expected = rust_i18n::t!("auth.required").into_owned();
        assert_eq!(result, BusinessResult::Message(expected.clone()));
        let published = output.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(extract_text(&published[0]), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn menu_authorized_returns_ok() {
        let service = ApplicationService;
        let output = RecordingOutput::new();
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        let result = service
            .handle(
                &command_input(PlatformId::Telegram, BusinessCommand::Menu),
                &state,
                &output,
            )
            .await
            .unwrap();
        assert_eq!(result, BusinessResult::Ok);
        assert!(output.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn menu_denied_identical_across_platforms() {
        let service = ApplicationService;
        for platform in [
            PlatformId::Telegram,
            PlatformId::Matrix,
            PlatformId::Discord,
        ] {
            let output = RecordingOutput::new();
            let state = make_state();
            let result = service
                .handle(
                    &command_input(platform, BusinessCommand::Menu),
                    &state,
                    &output,
                )
                .await
                .unwrap();
            let expected = rust_i18n::t!("auth.required").into_owned();
            assert_eq!(result, BusinessResult::Message(expected.clone()));
            let published = output.published.lock().unwrap();
            assert_eq!(published.len(), 1);
            assert_eq!(extract_text(&published[0]), Some(expected.as_str()));
        }
    }

    #[tokio::test]
    async fn menu_authorized_identical_across_platforms() {
        let service = ApplicationService;
        for platform in [
            PlatformId::Telegram,
            PlatformId::Matrix,
            PlatformId::Discord,
        ] {
            let output = RecordingOutput::new();
            let state = make_state();
            state.record_auth_success(42, Instant::now()).await;
            let result = service
                .handle(
                    &command_input(platform, BusinessCommand::Menu),
                    &state,
                    &output,
                )
                .await
                .unwrap();
            assert_eq!(result, BusinessResult::Ok);
            assert!(output.published.lock().unwrap().is_empty());
        }
    }
}
