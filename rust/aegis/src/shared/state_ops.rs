use crate::app::state::AppState;
use crate::bootstrap::BotSettings;
use crate::core::i18n;
use crate::shared::types::CallbackEvent;

use std::time::Instant;

#[allow(dead_code)]
pub async fn intercept(cb: &CallbackEvent, state: &AppState) {
    let data = cb.data.as_str();

    if data.starts_with("lang:") {
        handle_lang(cb, state).await;
        return;
    }

    if data.starts_with("set_timeout:") {
        handle_set_timeout(cb, state).await;
        return;
    }

    if data == "a_warp_add_input" {
        state
            .start_warp_input(cb.target.0.clone(), Instant::now())
            .await;
    }
}

async fn handle_set_timeout(cb: &CallbackEvent, state: &AppState) {
    let secs: u64 = cb
        .data
        .strip_prefix("set_timeout:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(600);
    state.set_session_timeout_secs(secs).await;
    let settings = BotSettings {
        session_timeout_secs: secs,
    };
    if let Err(e) = settings.save() {
        log::error!("保存会话设置失败: {}", e);
    }
}

async fn handle_lang(cb: &CallbackEvent, state: &AppState) {
    let lang = match cb.data.as_str() {
        "lang:zh" => i18n::Lang::Zh,
        "lang:en" => i18n::Lang::En,
        "lang:ja" => i18n::Lang::Ja,
        _ => return,
    };
    i18n::set_lang(lang);
    state.set_lang(lang).await;
    state.mark_lang_configured().await;
    i18n::mark_lang_configured();
    // Note: timedatectl and apt-daily timer stay in Telegram layer
    // (system operations that don't belong in shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::adapters::common::{BotAdapter, MessageId, MockBotAdapter, TargetId};
    use crate::app::state::AppState;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use crate::shared::types::CallbackEvent;

    use futures_util::future::BoxFuture;
    use std::sync::Arc;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(
                TotpManager::generate_new_secret(),
            ))
            .unwrap(),
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(MockBotAdapter::new()),
        )
    }

    #[tokio::test]
    async fn intercept_set_timeout_persists() {
        let state = make_state();
        let event = CallbackEvent {
            adapter: Arc::new(MockBotAdapter::new()) as Arc<dyn BotAdapter>,
            target: TargetId("123".into()),
            user_id: "42".into(),
            msg_id: MessageId("1".into()),
            data: "set_timeout:3600".into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        };
        intercept(&event, &state).await;
        assert_eq!(state.session_timeout_secs().await, 3600);
    }
}
