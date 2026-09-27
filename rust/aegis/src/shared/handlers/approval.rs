use crate::app::state::AppState;
use crate::common::{MessageContent, TargetId};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;

/// TG 带外审批回调：`sx_approve:<contactRequestId>` / `sx_reject:<contactRequestId>`。
///
/// 授权由 `check_auth` 提前承担：回调与命令同路，要求 Telegram 管理员已有活跃 TOTP
/// 会话（见 `shared/dispatch.rs`）。因此「点一下即批准」= 已经需要 TOTP，无需新机制。
/// onboarding 与「删联系人重加」后自动重绑：批准成功即把 contactId 记为管理员。
///
/// 判据是 **contactId 是否变化**，而不是「是否为空」。删掉 bot 联系人重连是最常见的
/// contactId 变更方式，而 `--tg-simplex` 下 SimpleX 入站全被
/// `classify_simplex_inbound` 丢弃，`app::auth` 的 SimpleX TOTP 自愈重钉根本走不到 ——
/// 若这里也不重绑，用户就永久卡死：落点钉在一个已删除的联系人上，自己却是新 contactId。
///
/// 安全性由既有 TOTP 会话承担：回调经 `check_auth`，能点批准的就是管理员本人。
/// 「多设备轮用会抢管理员」的顾虑不成立 —— 每次批准都要人在 TG 前操作。
///
/// 返回是否发生了重绑（供调用方决定文案 / 是否发安全码）。
fn auto_bind_on_approve(state: &AppState, contact_id: Option<i64>) -> bool {
    let Some(contact_id) = contact_id else {
        return false;
    };
    if state.simplex_admin_id() == Some(contact_id) {
        return false;
    }
    let previous = state.simplex_admin_id();
    // 先更新内存态：落盘失败也不该让「已批准却仍无权限」。
    state.set_simplex_admin_id(contact_id);
    // 敏感内容落点 + 敏感路由开关都在这一步恢复（见 RoutingAdapter::set_secondary_target），
    // 否则 onboarding 结束后敏感内容还被钉在 TG。
    state
        .adapter
        .set_secondary_target(TargetId(contact_id.to_string()));
    match crate::bootstrap::set_simplex_admin_id(&crate::bootstrap::config_dir(), contact_id) {
        Ok(()) => {
            log::warn!(
                "SimpleX 管理员已自动重绑: contactId={contact_id}（原 {previous:?}），已落盘"
            )
        }
        Err(e) => {
            log::error!("SimpleX 管理员已重绑为 contactId={contact_id}（仅内存态）；落盘失败: {e}")
        }
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
            Ok(contact_id) => {
                let bound = auto_bind_on_approve(state, contact_id);
                if bound {
                    // 绑定即发安全码：`set_secondary_target` 一执行，敏感内容就已开始
                    // 流向这条 SimpleX 连接，而未验证的连接挡不住 MITM。
                    crate::app::auth::notify_security_code(state).await;
                }
                (bound, Ok(()))
            }
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

    /// onboarding（未配管理员）时批准 → 自动绑定 + 热重钉落点 + **当场发安全码**。
    /// 安全码必须在绑定时发：`set_secondary_target` 一执行，敏感内容就开始流向
    /// SimpleX，而未验证的连接挡不住 MITM。等下一次 TOTP 登录才发，中间是空窗。
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
        mock.expect_contact_security_code()
            .times(1)
            .withf(|id| *id == 6)
            .returning(|_| Ok("52075 05398 87241 67434".to_string()));
        mock.expect_send_message_primary()
            .times(1)
            .withf(|target, content| {
                target.0 == "42" && content.text.contains("52075 05398 87241 67434")
            })
            .returning(|_, _| Ok(MessageId("1".into())));
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

    /// 删掉 bot 联系人重连 → contactId 变化 → 必须重绑。
    /// 这是 contactId 变更最常见的方式，而 `--tg-simplex` 下没有别的恢复通道
    /// （SimpleX 入站全被丢弃，SimpleX TOTP 自愈重钉走不到）。不重绑就永久卡死。
    #[tokio::test]
    async fn approve_rebinds_when_contact_changed() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(Some(6)));
        mock.expect_set_secondary_target()
            .withf(|t: &TargetId| t.0 == "6")
            .times(1)
            .returning(|_| ());
        mock.expect_contact_security_code()
            .times(1)
            .withf(|id| *id == 6)
            .returning(|_| Ok("52075 05398 87241 67434".to_string()));
        mock.expect_send_message_primary()
            .times(1)
            .returning(|_, _| Ok(MessageId("1".into())));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .withf(|_, _, c: &MessageContent| c.text == t!("simplex.approval_done_bound").as_ref())
            .returning(|_, _, _| Ok(()));
        let adapter: Arc<dyn crate::common::BotAdapter> = Arc::new(mock);
        let state = test_state(adapter.clone(), Some(5));
        let event = CallbackEvent {
            adapter,
            ..event_with("sx_approve:2")
        };
        handle(&event, &state).await.unwrap();
        assert_eq!(state.simplex_admin_id(), Some(6), "contactId 变化必须重绑");
    }

    /// contactId **未变**（重复批准同一个）—— 不得重绑、不得发码。
    #[tokio::test]
    async fn approve_does_not_rebind_when_contact_unchanged() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(Some(7)));
        mock.expect_set_secondary_target().times(0);
        mock.expect_contact_security_code().times(0);
        mock.expect_send_message_primary().times(0);
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
        assert_eq!(state.simplex_admin_id(), Some(7));
    }

    /// 适配器拿不到 contactId（返回 None）时不得误判为绑定成功。
    #[tokio::test]
    async fn approve_without_contact_id_leaves_admin_unset() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(None));
        mock.expect_set_secondary_target().times(0);
        mock.expect_contact_security_code().times(0);
        mock.expect_send_message_primary().times(0);
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
