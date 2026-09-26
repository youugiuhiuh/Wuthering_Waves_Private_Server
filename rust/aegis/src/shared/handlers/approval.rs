use crate::app::state::AppState;
use crate::common::{MessageContent, TargetId};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;

/// TG 带外审批回调：`sx_approve:<contactRequestId>` / `sx_reject:<contactRequestId>`。
///
/// 授权由 `check_auth` 提前承担：回调与命令同路，要求 Telegram 管理员已有活跃 TOTP
/// 会话（见 `shared/dispatch.rs`）。因此「点一下即批准」= 已经需要 TOTP，无需新机制。
/// onboarding（`simplex_admin_id` 未配）时，批准成功即自动绑定：SDK 的
/// `AcceptingContactRequestResponse` 带着刚接受出来的 contactId，扔掉它等于逼用户
/// 去翻日志 + 手动 `--set-simplex-admin` + 重启。
///
/// 保守：仅在**尚未配置管理员**时绑定。已绑定时批准新联系人不碰配置 ——
/// 否则「谁最后被批准谁是管理员」，多设备轮用会在管理员身份之间来回跳。
///
/// 返回是否发生了绑定（供调用方决定文案）。
fn auto_bind_on_approve(state: &AppState, contact_id: Option<i64>) -> bool {
    let Some(contact_id) = contact_id else {
        return false;
    };
    if state.simplex_admin_id().is_some() {
        return false;
    }
    // 先更新内存态：落盘失败也不该让「已批准却仍无权限」。
    state.set_simplex_admin_id(contact_id);
    // 敏感内容落点 + 敏感路由开关都在这一步恢复（见 RoutingAdapter::set_secondary_target），
    // 否则 onboarding 结束后敏感内容还被钉在 TG。
    state
        .adapter
        .set_secondary_target(TargetId(contact_id.to_string()));
    match crate::bootstrap::set_simplex_admin_id(&crate::bootstrap::config_dir(), contact_id) {
        Ok(()) => {
            log::warn!("onboarding 完成：SimpleX 管理员已自动绑定 contactId={contact_id}，已落盘")
        }
        Err(e) => log::error!(
            "onboarding 完成：SimpleX 管理员已绑定 contactId={contact_id}（仅内存态）；落盘失败: {e}"
        ),
    }
    true
}

pub(crate) async fn handle(event: &CallbackEvent, state: &AppState) -> HandlerResult {
    let (approve, raw_id) = match event.data.split_once(':') {
        Some(("sx_approve", id)) => (true, id),
        Some(("sx_reject", id)) => (false, id),
        _ => return Ok(HandlerAction::Done),
    };
    let Ok(id) = raw_id.parse::<i64>() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("simplex.bad_request_id").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    let (bound_contact_id, result) = if approve {
        match event.adapter.accept_contact_request(id).await {
            Ok(contact_id) => (auto_bind_on_approve(state, contact_id), Ok(())),
            Err(e) => (false, Err(e)),
        }
    } else {
        (false, event.adapter.reject_contact_request(id).await)
    };

    let (text, answer) = match result {
        Ok(()) => (
            if bound_contact_id {
                t!("simplex.approval_done_bound").into_owned()
            } else {
                t!("simplex.approval_done", "0" => id.to_string()).into_owned()
            },
            if approve {
                t!("simplex.approved").into_owned()
            } else {
                t!("simplex.rejected").into_owned()
            },
        ),
        Err(e) => {
            log::error!("SimpleX 联系人审批失败 (id={id}, approve={approve}): {e}");
            (
                t!("simplex.approval_failed", "0" => e.to_string()).into_owned(),
                t!("simplex.approval_failed_short").into_owned(),
            )
        }
    };

    event
        .adapter
        .answer_callback(&event.target, &event.callback_id, Some(answer))
        .await?;
    // 用 edit 替换原通知，顺带撤掉 [允许][拒绝] 按钮：第二次点已处理的 id 会报错，
    // 看起来像故障。编辑失败（消息过旧等）不致命，退回 send_message 保证结果可见。
    let content = MessageContent { text, markup: None };
    if event.msg_id.0.is_empty()
        || event
            .adapter
            .edit_message(&event.target, &event.msg_id, content.clone())
            .await
            .is_err()
    {
        event.adapter.send_message(&event.target, content).await?;
    }
    Ok(HandlerAction::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{MessageContent, MessageId, MockBotAdapter, TargetId};
    use std::sync::Arc;

    /// 不执行任何自愈副作用的执行器（自动绑定会走落盘；测试只关心内存态）。
    struct NoopExecutor;
    impl crate::core::security::self_destruct::SelfDestructExecutor for NoopExecutor {
        fn execute(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    /// state 用与事件同一个 adapter —— 自动绑定要经 `state.adapter.set_secondary_target`
    /// 热重钉落点，两者必须是同一个对象。
    fn test_state(adapter: Arc<dyn crate::common::BotAdapter>, sx_admin: Option<i64>) -> AppState {
        AppState::new(
            Some(42),
            sx_admin,
            None,
            Arc::new(NoopExecutor),
            None,
            600,
            adapter,
        )
    }

    fn event_with(data: &str) -> CallbackEvent {
        CallbackEvent {
            adapter: Arc::new(MockBotAdapter::new()),
            target: TargetId("42".into()),
            user_id: "42".into(),
            msg_id: MessageId("1".into()),
            data: data.into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        }
    }

    fn mock_event(mock: MockBotAdapter, data: &str) -> CallbackEvent {
        CallbackEvent {
            adapter: Arc::new(mock),
            ..event_with(data)
        }
    }

    /// onboarding（未配管理员）时批准 → 自动绑定 + 热重钉落点。
    #[tokio::test]
    async fn approve_auto_binds_admin_during_onboarding() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(Some(6)));
        mock.expect_set_secondary_target()
            .withf(|t: &TargetId| t.0 == "6")
            .times(1)
            .returning(|_| ());
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .withf(|_, _, c: &MessageContent| c.text == t!("simplex.approval_done_bound").as_ref())
            .returning(|_, _, _| Ok(()));
        let adapter: Arc<dyn crate::common::BotAdapter> = Arc::new(mock);
        let state = test_state(adapter.clone(), None);
        let event = CallbackEvent {
            adapter,
            ..event_with("sx_approve:2")
        };
        handle(&event, &state).await.unwrap();
        assert_eq!(
            state.simplex_admin_id(),
            Some(6),
            "onboarding 批准后必须自动绑定 contactId"
        );
    }

    /// 已有管理员时批准新联系人 —— 不得改配置（否则多设备轮用会来回抢管理员）。
    #[tokio::test]
    async fn approve_does_not_rebind_when_admin_already_set() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(Some(99)));
        // 落点不得被重钉。
        mock.expect_set_secondary_target().times(0);
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .withf(|_, _, c: &MessageContent| c.text != t!("simplex.approval_done_bound").as_ref())
            .returning(|_, _, _| Ok(()));
        let adapter: Arc<dyn crate::common::BotAdapter> = Arc::new(mock);
        let state = test_state(adapter.clone(), Some(7));
        let event = CallbackEvent {
            adapter,
            ..event_with("sx_approve:2")
        };
        handle(&event, &state).await.unwrap();
        assert_eq!(state.simplex_admin_id(), Some(7), "已有管理员不得被覆盖");
    }

    /// 适配器拿不到 contactId（返回 None）时不得误判为绑定成功。
    #[tokio::test]
    async fn approve_without_contact_id_leaves_admin_unset() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(None));
        mock.expect_set_secondary_target().times(0);
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .withf(|_, _, c: &MessageContent| c.text != t!("simplex.approval_done_bound").as_ref())
            .returning(|_, _, _| Ok(()));
        let adapter: Arc<dyn crate::common::BotAdapter> = Arc::new(mock);
        let state = test_state(adapter.clone(), None);
        let event = CallbackEvent {
            adapter,
            ..event_with("sx_approve:2")
        };
        handle(&event, &state).await.unwrap();
        assert_eq!(state.simplex_admin_id(), None);
    }

    #[tokio::test]
    async fn approve_callback_calls_adapter() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .with(mockall::predicate::eq(7))
            .times(1)
            .returning(|_| Ok(None));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let event = mock_event(mock, "sx_approve:7");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    #[tokio::test]
    async fn reject_callback_calls_adapter() {
        let mut mock = MockBotAdapter::new();
        mock.expect_reject_contact_request()
            .with(mockall::predicate::eq(8))
            .times(1)
            .returning(|_| Ok(()));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let event = mock_event(mock, "sx_reject:8");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    /// 适配器报错时不得 panic，必须回一条失败消息（并 answer 回调查询）。
    #[tokio::test]
    async fn approval_failure_reports_and_answers() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| anyhow::bail!("boom"));
        mock.expect_answer_callback()
            .times(1)
            .returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let event = mock_event(mock, "sx_approve:9");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    /// edit 失败（消息过旧）时退回 send_message，结果仍可见。
    #[tokio::test]
    async fn approval_falls_back_to_send_when_edit_fails() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(None));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| anyhow::bail!("message is too old"));
        mock.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("2".into())));
        let event = mock_event(mock, "sx_approve:7");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    #[tokio::test]
    async fn malformed_id_answers_but_does_not_call_adapter() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request().times(0);
        mock.expect_answer_callback()
            .times(1)
            .returning(|_, _, _| Ok(()));
        mock.expect_edit_message().times(0);
        mock.expect_send_message().times(0);
        let event = mock_event(mock, "sx_approve:abc");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    #[tokio::test]
    async fn unrelated_data_is_ignored() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request().times(0);
        mock.expect_answer_callback().times(0);
        mock.expect_edit_message().times(0);
        mock.expect_send_message().times(0);
        let event = mock_event(mock, "m_main");
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }

    /// 空 msg_id（无原消息可编辑）时直接 send。
    #[tokio::test]
    async fn empty_msg_id_falls_back_to_send() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(None));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message().times(0);
        mock.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("2".into())));
        let mut event = mock_event(mock, "sx_approve:7");
        event.msg_id = MessageId(String::new());
        assert_eq!(
            handle(&event, &test_state(event.adapter.clone(), Some(7)))
                .await
                .unwrap(),
            HandlerAction::Done
        );
    }
}
