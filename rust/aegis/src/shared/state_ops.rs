use crate::app::state::AppState;
use crate::bootstrap::BotSettings;
use crate::core::i18n;
use crate::shared::types::CallbackEvent;

use std::time::Instant;

#[allow(dead_code)]
pub async fn intercept(cb: &CallbackEvent, state: &AppState) -> Option<String> {
    let data = cb.data.as_str();

    if data.starts_with("lang:") {
        return handle_lang(cb, state).await;
    }

    if data.starts_with("set_timeout:") {
        handle_set_timeout(cb, state).await;
        return None;
    }

    if data == "a_warp_add_input" {
        state
            .start_warp_input(cb.target.0.clone(), Instant::now())
            .await;
    }

    if data == "a_one_click_domain" || data == "u_xhttp_domain" {
        state.start_domain_input(cb.target.0.clone()).await;
    }

    if data.starts_with("xhttp_tls_prov:") {
        use crate::app::state::DomainInputStep;
        use crate::core::types::DnsProvider;
        let provider = match data.strip_prefix("xhttp_tls_prov:") {
            Some("cf") => DnsProvider::Cloudflare,
            Some("ali") => DnsProvider::Aliyun,
            Some("dp") => DnsProvider::Dnspod,
            Some("aws") => DnsProvider::Route53,
            _ => return None,
        };
        state
            .update_domain_input(&cb.target.0, |s| {
                s.step = DomainInputStep::AwaitCredentials(provider);
            })
            .await;
    }

    None
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

async fn handle_lang(cb: &CallbackEvent, state: &AppState) -> Option<String> {
    let lang = match cb.data.as_str() {
        "lang:zh" => i18n::Lang::Zh,
        "lang:en" => i18n::Lang::En,
        "lang:ja" => i18n::Lang::Ja,
        _ => return None,
    };
    i18n::set_lang(lang);
    state.set_lang(lang).await;
    if let Err(e) = crate::bootstrap::save_lang_to_config(lang) {
        log::error!("保存语言配置失败: {}", e);
    }
    state.mark_lang_configured().await;
    i18n::mark_lang_configured();
    if let Err(e) = cb.adapter.set_system_locale(lang).await {
        log::error!("设置系统语言环境失败: {}", e);
    }
    if let Err(e) = cb
        .adapter
        .answer_callback(
            &cb.target,
            &cb.callback_id,
            Some(rust_i18n::t!("lang.switched", "0" => lang.as_str()).to_string()),
        )
        .await
    {
        log::error!("语言切换回调确认失败: {}", e);
    }
    Some("m_main".into())
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
            None,
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
