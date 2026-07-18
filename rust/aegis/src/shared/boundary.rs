use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::common::{MessageContent, Platform};
use crate::app::state::AppState;
use crate::shared::error::DispatchError;
use crate::shared::types::BotEvent;

pub struct EventContext {
    pub kind: &'static str,
    pub platform: Platform,
    pub user_id: i64,
    pub target: String,
}

impl EventContext {
    pub fn from_event(event: &BotEvent) -> Self {
        let kind = match event {
            BotEvent::Message(_) => "message",
            BotEvent::Command(_) => "command",
            BotEvent::Callback(_) => "callback",
        };
        Self {
            kind,
            platform: event.adapter().platform(),
            user_id: event.user_id(),
            target: event.target().0.clone(),
        }
    }
}

fn generate_event_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:08x}{:04x}", ts & 0xFFFF_FFFF, count & 0xFFFF)
}

pub async fn handle_dispatch_result(
    state: &AppState,
    ctx: &EventContext,
    result: Result<(), DispatchError>,
) {
    let Err(error) = result else { return };

    let event_id = generate_event_id();

    log::error!(
        "[{event_id}] dispatch failed: platform={:?} kind={} user={} target={} error={error:#}",
        ctx.platform,
        ctx.kind,
        ctx.user_id,
        ctx.target,
    );

    let error_text = rust_i18n::t!("internal_error");
    let msg = format!("{error_text}\nEvent ID: {event_id}");

    if let Err(send_err) = state
        .adapter
        .send_message(
            &crate::adapters::common::TargetId(ctx.target.clone()),
            MessageContent {
                text: msg,
                markup: None,
            },
        )
        .await
    {
        log::error!("[{event_id}] failed to notify dispatch error: {send_err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::{
        BotAdapter, MessageContent, MessageId, Platform, Principal, TargetId,
    };
    use crate::app::state::AppState;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use crate::shared::error::DispatchError;
    use crate::shared::types::{BotEvent, CallbackEvent, CommandEvent, MessageEvent};
    use anyhow::anyhow;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use std::sync::Arc;

    struct RecordingAdapter {
        pub messages: std::sync::Mutex<Vec<String>>,
        pub platform: Platform,
    }

    #[async_trait]
    impl BotAdapter for RecordingAdapter {
        fn platform(&self) -> Platform {
            self.platform
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            self.messages.lock().unwrap().push(content.text);
            Ok(MessageId("0".into()))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn capabilities(&self) -> crate::adapters::common::PlatformCapabilities {
            crate::adapters::common::PlatformCapabilities::TELEGRAM
        }
    }

    struct SenderFailsAdapter;

    #[async_trait]
    impl BotAdapter for SenderFailsAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            Err(anyhow!("send failure"))
        }
        async fn edit_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
            _content: MessageContent,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_message(
            &self,
            _target: &TargetId,
            _msg_id: &MessageId,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn capabilities(&self) -> crate::adapters::common::PlatformCapabilities {
            crate::adapters::common::PlatformCapabilities::TELEGRAM
        }
    }

    struct NoopExecutor;
    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn make_state(adapter: Arc<dyn BotAdapter>) -> Arc<AppState> {
        Arc::new(AppState::new(
            42,
            None,
            TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
            adapter,
        ))
    }

    fn test_event(adapter: Arc<dyn BotAdapter>) -> BotEvent {
        BotEvent::Message(MessageEvent {
            adapter,
            target: TargetId("test-123".into()),
            principal: Principal::telegram(42),
            text: Some("hello".into()),
            file_id: None,
            file_name: None,
            reply_to_text: None,
        })
    }

    #[tokio::test]
    async fn event_context_extracts_platform_and_user() {
        let adapter = Arc::new(RecordingAdapter {
            messages: std::sync::Mutex::new(vec![]),
            platform: Platform::Telegram,
        });
        let event = test_event(adapter.clone());
        let ctx = EventContext::from_event(&event);
        assert_eq!(ctx.platform, Platform::Telegram);
        assert_eq!(ctx.user_id, 42);
        assert_eq!(ctx.kind, "message");
        assert!(ctx.target.contains("test-123"));
    }

    #[tokio::test]
    async fn event_context_kind_for_command() {
        let adapter = Arc::new(RecordingAdapter {
            messages: std::sync::Mutex::new(vec![]),
            platform: Platform::Matrix,
        });
        let event = BotEvent::Command(CommandEvent {
            adapter,
            target: TargetId("r".into()),
            principal: Principal::matrix("@u:r").unwrap(),
            command: crate::shared::types::BotCommand::Help,
        });
        let ctx = EventContext::from_event(&event);
        assert_eq!(ctx.kind, "command");
        assert_eq!(ctx.platform, Platform::Matrix);
    }

    #[tokio::test]
    async fn event_context_kind_for_callback() {
        let adapter = Arc::new(RecordingAdapter {
            messages: std::sync::Mutex::new(vec![]),
            platform: Platform::Discord,
        });
        let event = BotEvent::Callback(CallbackEvent {
            adapter,
            target: TargetId("c".into()),
            principal: Principal::discord(7),
            msg_id: MessageId("m".into()),
            data: "cb_data".into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        });
        let ctx = EventContext::from_event(&event);
        assert_eq!(ctx.kind, "callback");
        assert_eq!(ctx.user_id, 7);
    }

    #[tokio::test]
    async fn success_does_not_send_message() {
        let adapter = Arc::new(RecordingAdapter {
            messages: std::sync::Mutex::new(vec![]),
            platform: Platform::Telegram,
        });
        let state = make_state(adapter.clone());
        let event = test_event(adapter.clone());
        let ctx = EventContext::from_event(&event);
        let _event = event; // consume original event
        handle_dispatch_result(&state, &ctx, Ok(())).await;
        let msgs = adapter.messages.lock().unwrap();
        assert!(msgs.is_empty(), "success must not send error message");
    }

    #[tokio::test]
    async fn failure_sends_one_message_with_event_id() {
        let adapter = Arc::new(RecordingAdapter {
            messages: std::sync::Mutex::new(vec![]),
            platform: Platform::Telegram,
        });
        let state = make_state(adapter.clone());
        let event = test_event(adapter.clone());
        let ctx = EventContext::from_event(&event);
        let dispatch_result: Result<(), DispatchError> = Err(anyhow!("oops").into());
        handle_dispatch_result(&state, &ctx, dispatch_result).await;
        let msgs = adapter.messages.lock().unwrap();
        assert_eq!(msgs.len(), 1, "failure must send exactly one message");
        assert!(
            msgs[0].contains("Event ID:"),
            "message must contain Event ID: got '{}'",
            msgs[0]
        );
    }

    #[tokio::test]
    async fn notification_failure_does_not_panic() {
        let state = make_state(Arc::new(SenderFailsAdapter));
        let event = test_event(Arc::new(SenderFailsAdapter));
        let ctx = EventContext::from_event(&event);
        let dispatch_result: Result<(), DispatchError> = Err(anyhow!("inner error").into());
        handle_dispatch_result(&state, &ctx, dispatch_result).await;
    }

    #[tokio::test]
    async fn generate_event_id_returns_non_empty_and_unique() {
        let a = generate_event_id();
        let b = generate_event_id();
        assert!(!a.is_empty());
        assert!(!b.is_empty());
        assert_ne!(a, b, "sequential calls must produce different IDs");
    }
}
