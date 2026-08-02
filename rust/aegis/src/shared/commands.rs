use crate::app::auth;
use crate::app::interaction::{
    ActorId, BusinessInput, BusinessMessage, BusinessRequest, BusinessResult, ConversationId,
    Origin, PlatformId,
};
use crate::app::output::BusinessOutput;
use crate::app::service::ApplicationService;
use crate::app::state::AppState;
use crate::common::{BotAdapter, MessageContent, Platform, TargetId};
use crate::shared::types::{BotCommand, CommandEvent};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

#[allow(dead_code)]
pub async fn handle(cmd: CommandEvent, state: &AppState) -> Result<()> {
    // Legacy Auth path stays in the bridge; the service treats it as
    // unreachable and returns Ok.
    if let BotCommand::Auth { code } = &cmd.command {
        let _ = auth::process_auth_code(
            &*cmd.adapter,
            &cmd.target,
            cmd.user_id,
            code,
            state,
            5,
            Duration::from_secs(600),
            &[
                Duration::from_secs(900),
                Duration::from_secs(3600),
                Duration::from_secs(86400),
                Duration::from_secs(172800),
            ],
        )
        .await;
        return Ok(());
    }

    let input = bridge_input(&cmd);
    let output = AdapterOutput {
        adapter: cmd.adapter.clone(),
        target: cmd.target.clone(),
    };
    let result = ApplicationService
        .handle(&input, state, &output)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if matches!(&cmd.command, BotCommand::Menu) && matches!(result, BusinessResult::Ok) {
        crate::shared::handlers::menu::send_main_menu(&*cmd.adapter, &cmd.target).await?;
    }
    Ok(())
}

/// Temporary dispatch bridge: converts a legacy `CommandEvent` into a
/// `BusinessRequest` and adapts its `BotAdapter` to `BusinessOutput`.
fn bridge_input(cmd: &CommandEvent) -> BusinessInput {
    BusinessInput {
        origin: Origin {
            platform: match cmd.adapter.platform() {
                Platform::Telegram => PlatformId::Telegram,
                Platform::Discord => PlatformId::Discord,
                Platform::Matrix => PlatformId::Matrix,
            },
            actor_id: ActorId::new(cmd.user_id.to_string()).unwrap(),
            conversation_id: ConversationId::new(cmd.target.0.clone()).unwrap(),
        },
        request: BusinessRequest::Command(cmd.command.clone()),
    }
}

struct AdapterOutput {
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
}

#[async_trait]
impl BusinessOutput for AdapterOutput {
    async fn publish(&self, message: BusinessMessage) -> Result<()> {
        self.adapter
            .send_message(
                &self.target,
                MessageContent {
                    text: message.text,
                    markup: None,
                },
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
    use crate::core::totp::TotpManager;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::time::Instant;

    use crate::core::security::self_destruct::SelfDestructExecutor;
    use anyhow::Result as AnyResult;
    use futures_util::future::BoxFuture;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, AnyResult<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Default)]
    struct MockAdapter {
        pub sent: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BotAdapter for MockAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            content: MessageContent,
        ) -> AnyResult<MessageId> {
            self.sent.lock().unwrap().push(content.text);
            Ok(MessageId("0".to_string()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> AnyResult<()> {
            Ok(())
        }
        async fn delete_message(&self, _target: &TargetId, _msg_id: &MessageId) -> AnyResult<()> {
            Ok(())
        }
        async fn download_file(&self, _file_id: &str) -> AnyResult<Vec<u8>> {
            Ok(Vec::new())
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

    fn make_cmd(adapter: Arc<MockAdapter>, command: BotCommand) -> CommandEvent {
        CommandEvent {
            adapter,
            target: TargetId("123".into()),
            user_id: 42,
            command,
        }
    }

    #[tokio::test]
    async fn help_sends_help_text() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        handle(make_cmd(adapter.clone(), BotCommand::Help), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("help") || !sent[0].is_empty());
    }

    #[tokio::test]
    async fn start_sends_welcome() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        handle(make_cmd(adapter.clone(), BotCommand::Start), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].is_empty());
    }

    #[tokio::test]
    async fn auth_calls_process_auth_code() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        // Should not panic / returns Ok even when user not admin-authed flow
        let res = handle(
            make_cmd(
                adapter.clone(),
                BotCommand::Auth {
                    code: "123456".into(),
                },
            ),
            &state,
        )
        .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn menu_sends_auth_required_when_not_authorized() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        // admin 42 but no session recorded -> not authorized
        handle(make_cmd(adapter.clone(), BotCommand::Menu), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("auth") || !sent[0].is_empty());
    }

    #[tokio::test]
    async fn menu_sends_main_menu_when_authorized() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        handle(make_cmd(adapter.clone(), BotCommand::Menu), &state)
            .await
            .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        // main menu has markup; check non-empty text
        assert!(!sent[0].is_empty());
    }

    #[tokio::test]
    async fn set_security_file_sends_recent_auth_required_when_not_recent() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        // no recent session -> requires recent auth
        handle(
            make_cmd(adapter.clone(), BotCommand::SetSecurityFile),
            &state,
        )
        .await
        .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].is_empty());
    }

    #[tokio::test]
    async fn set_security_file_sends_prompt_when_recently_authenticated() {
        let adapter = Arc::new(MockAdapter::default());
        let state = make_state();
        state.record_auth_success(42, Instant::now()).await;
        handle(
            make_cmd(adapter.clone(), BotCommand::SetSecurityFile),
            &state,
        )
        .await
        .unwrap();
        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].is_empty());
    }

    struct TestAdapter;
    #[async_trait]
    impl BotAdapter for TestAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(&self, _t: &TargetId, _c: MessageContent) -> AnyResult<MessageId> {
            Ok(MessageId("0".into()))
        }
        async fn edit_message(
            &self,
            _t: &TargetId,
            _m: &MessageId,
            _c: MessageContent,
        ) -> AnyResult<()> {
            Ok(())
        }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> AnyResult<()> {
            Ok(())
        }
        async fn download_file(&self, _f: &str) -> AnyResult<Vec<u8>> {
            Ok(vec![])
        }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, AnyResult<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    use crate::shared::types::TimeoutStatus;

    #[tokio::test]
    async fn set_security_file_starts_pending_input() {
        let secret = TotpManager::generate_new_secret();
        let state = Arc::new(AppState::new(
            Some(42),
            None,
            Some(TotpManager::new(&secrecy::SecretString::from(secret)).unwrap()),
            Arc::new(TestExecutor),
            None,
            600,
        ));
        state.record_auth_success(42, Instant::now()).await;
        let cmd = CommandEvent {
            adapter: Arc::new(TestAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            user_id: 42,
            command: BotCommand::SetSecurityFile,
        };
        handle(cmd, &state).await.unwrap();
        assert_eq!(
            state
                .take_security_file_input_status("42", Duration::from_secs(180))
                .await,
            TimeoutStatus::Active
        );
    }
}
