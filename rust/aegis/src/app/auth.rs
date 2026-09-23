use std::time::{Duration, Instant};

use aegis::common::{BotAdapter, InlineButton, Markup, MessageContent, Platform, TargetId};
use anyhow::Result;
use rust_i18n::t;

use crate::app::state::{AppState, AuthFailureOutcome};

#[allow(clippy::too_many_arguments)]
pub async fn process_auth_code(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    user_id: i64,
    code: &str,
    state: &AppState,
    max_attempts: u32,
    failure_window: Duration,
    lockout_durations: &[Duration],
) -> Result<bool> {
    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("auth.rate_limit", "0" => (remaining.as_secs() / 60).to_string(), "1" => (remaining.as_secs() % 60).to_string()).to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;

        // 自愈：只有**来自 SimpleX** 的 TOTP 成功才重钉 simplex_admin_id。
        // `is_admin_user` 的 user_id 命名空间与 Telegram 共用，若不加平台判据，
        // `--tg-simplex` 下 TG 管理员的码会把 simplex_admin_id 覆写成 TG chat id，
        // 破坏「敏感内容落点」。平台判据取自事件自身的 adapter（无需改调用点）。
        if state.simplex_repin_enabled() && adapter.platform() == Platform::Simplex {
            let previous = state.simplex_admin_id();
            // 先更新内存态：即使落盘失败，本次运行也已可用，避免「验过码却还是没权限」。
            state.set_simplex_admin_id(user_id);
            // 敏感内容落点也是运行时值，必须同步刷新；否则 tg-simplex 下仍发往
            // 启动快照里的旧 contactId（要重启才生效）。
            state
                .adapter
                .set_secondary_target(TargetId(user_id.to_string()));
            match crate::bootstrap::set_simplex_admin_id(&crate::bootstrap::config_dir(), user_id) {
                Ok(()) => log::warn!(
                    "SimpleX 管理员已重钉: contactId={user_id}（原 {previous:?}），已落盘"
                ),
                Err(e) => log::error!(
                    "SimpleX 管理员已重钉为 contactId={user_id}（仅内存态）；落盘失败: {e}"
                ),
            }
        }

        let success_text =
            t!("auth.success", "0" => crate::utils::format_duration_human(timeout)).to_string();
        if !state.is_lang_configured().await {
            let lang_text = t!("welcome.select_language").to_string();
            let lang_markup = Markup {
                buttons: vec![vec![
                    InlineButton {
                        text: t!("lang.zh").to_string(),
                        data: "lang:zh".to_string(),
                    },
                    InlineButton {
                        text: t!("lang.en").to_string(),
                        data: "lang:en".to_string(),
                    },
                    InlineButton {
                        text: t!("lang.ja").to_string(),
                        data: "lang:ja".to_string(),
                    },
                ]],
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: format!("{}\n\n{}", success_text, lang_text),
                        markup: Some(lang_markup),
                    },
                )
                .await?;
        } else {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: success_text,
                        markup: None,
                    },
                )
                .await?;
        }
        return Ok(true);
    }

    match state
        .record_auth_failure(
            user_id,
            now,
            max_attempts,
            failure_window,
            lockout_durations,
        )
        .await
    {
        AuthFailureOutcome::Locked { duration } => {
            let duration_str = if duration.as_secs() >= 3600 {
                format!("{} {}", duration.as_secs() / 3600, t!("auth.hours"))
            } else {
                format!("{} {}", duration.as_secs() / 60, t!("auth.minutes"))
            };
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("auth.locked", "0" => duration_str).to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
        AuthFailureOutcome::Invalid {
            attempts,
            max_attempts,
        } => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("auth.invalid_code", "0" => attempts.to_string(), "1" => max_attempts.to_string()).to_string(),
                        markup: None,
                    },
                )
                .await?;
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::common::{MessageId, MockBotAdapter, Platform, PlatformCapabilities};
    use aegis::core::security::self_destruct::SelfDestructExecutor;
    use aegis::core::totp::TotpManager;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;
    use std::sync::Arc;

    struct NoopExecutor;

    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct RecordingAdapter;

    #[async_trait::async_trait]
    impl BotAdapter for RecordingAdapter {
        fn platform(&self) -> Platform {
            Platform::Simplex
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            Ok(MessageId("0".to_string()))
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
        async fn download_file(&self, _file_id: &str) -> anyhow::Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities::SIMPLEX
        }
    }

    struct TelegramRecordingAdapter;

    #[async_trait::async_trait]
    impl BotAdapter for TelegramRecordingAdapter {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }
        async fn send_message(
            &self,
            _target: &TargetId,
            _content: MessageContent,
        ) -> anyhow::Result<MessageId> {
            Ok(MessageId("0".to_string()))
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
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities::TELEGRAM
        }
    }

    /// 空 config 目录：让 `set_simplex_admin_id` 读不到 config.enc 而失败，
    /// 从而验证「落盘失败不丢内存态」。
    fn state_with_repin_and(
        repin: bool,
        adapter: Arc<dyn BotAdapter>,
    ) -> (AppState, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        // SAFETY: 测试进程内串行修改环境变量。本模块测试均在同一进程且不并发读写
        // AEGIS_CONFIG_DIR（nextest 默认每个测试独立进程）。
        unsafe { std::env::set_var("AEGIS_CONFIG_DIR", dir.path()) };
        let state = AppState::new(
            None,
            Some(3),
            Some(
                TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
            ),
            Arc::new(NoopExecutor),
            None,
            600,
            adapter,
        );
        let state = if repin {
            state.with_simplex_repin()
        } else {
            state
        };
        (state, dir)
    }

    fn state_with_repin(repin: bool) -> (AppState, tempfile::TempDir) {
        state_with_repin_and(repin, Arc::new(RecordingAdapter))
    }

    async fn run_code(state: &AppState, user_id: i64, code: &str) -> bool {
        process_auth_code(
            &RecordingAdapter,
            &TargetId(user_id.to_string()),
            user_id,
            code,
            state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap_or(false)
    }

    #[tokio::test]
    async fn successful_code_repins_simplex_admin_when_enabled() {
        let (state, _dir) = state_with_repin(true);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        assert!(run_code(&state, 7, &code).await);
        assert_eq!(
            state.simplex_admin_id(),
            Some(7),
            "验证成功后必须把管理员重钉到本身份；即使落盘失败也要本次可用"
        );
        assert!(state.is_admin_user(7));
    }

    #[tokio::test]
    async fn successful_code_does_not_repin_when_disabled() {
        let (state, _dir) = state_with_repin(false);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        assert!(run_code(&state, 7, &code).await);
        assert_eq!(
            state.simplex_admin_id(),
            Some(3),
            "未开启重钉时（tg-simplex）绝不能被改动"
        );
    }

    #[tokio::test]
    async fn wrong_code_does_not_repin() {
        let (state, _dir) = state_with_repin(true);
        assert!(!run_code(&state, 7, "000000").await);
        assert_eq!(state.simplex_admin_id(), Some(3), "错码不得改动管理员");
    }

    /// TG 发来的码绝不改 simplex_admin_id —— 否则敏感内容落点会被覆写成 TG chat id。
    #[tokio::test]
    async fn telegram_code_does_not_repin_simplex_admin() {
        let (state, _dir) = state_with_repin(true);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        let ok = process_auth_code(
            &TelegramRecordingAdapter,
            &TargetId("42".into()),
            42,
            &code,
            &state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap();
        assert!(ok, "TG 验证仍应成功建会话");
        assert_eq!(
            state.simplex_admin_id(),
            Some(3),
            "TG 不得重钉 SimpleX 管理员"
        );
    }

    /// 重钉必须同时刷新 `state.adapter` 的敏感内容落点；否则 tg-simplex 下敏感内容
    /// 仍发往启动快照里的旧 contactId（真机实测踩到）。
    #[tokio::test]
    async fn repin_refreshes_sensitive_secondary_target() {
        let mut mock = MockBotAdapter::new();
        mock.expect_set_secondary_target()
            .withf(|t| t.0 == "7")
            .times(1)
            .returning(|_| ());
        let (state, _dir) = state_with_repin_and(true, Arc::new(mock));
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        let ok = process_auth_code(
            &RecordingAdapter,
            &TargetId("7".into()),
            7,
            &code,
            &state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap();
        assert!(ok);
    }
}
