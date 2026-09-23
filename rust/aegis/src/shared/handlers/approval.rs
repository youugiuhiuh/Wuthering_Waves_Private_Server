use crate::common::MessageContent;
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;

/// TG 带外审批回调：`sx_approve:<contactRequestId>` / `sx_reject:<contactRequestId>`。
///
/// 授权由 `check_auth` 提前承担：回调与命令同路，要求 Telegram 管理员已有活跃 TOTP
/// 会话（见 `shared/dispatch.rs`）。因此「点一下即批准」= 已经需要 TOTP，无需新机制。
pub(crate) async fn handle(event: &CallbackEvent) -> HandlerResult {
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

    let result = if approve {
        event.adapter.accept_contact_request(id).await
    } else {
        event.adapter.reject_contact_request(id).await
    };

    let (text, answer) = match result {
        Ok(()) => (
            t!("simplex.approval_done", "0" => id.to_string()).into_owned(),
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
    use crate::common::{MessageId, MockBotAdapter, TargetId};
    use std::sync::Arc;

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

    #[tokio::test]
    async fn approve_callback_calls_adapter() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .with(mockall::predicate::eq(7))
            .times(1)
            .returning(|_| Ok(()));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let event = mock_event(mock, "sx_approve:7");
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
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
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
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
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
    }

    /// edit 失败（消息过旧）时退回 send_message，结果仍可见。
    #[tokio::test]
    async fn approval_falls_back_to_send_when_edit_fails() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(()));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message()
            .times(1)
            .returning(|_, _, _| anyhow::bail!("message is too old"));
        mock.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("2".into())));
        let event = mock_event(mock, "sx_approve:7");
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
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
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
    }

    #[tokio::test]
    async fn unrelated_data_is_ignored() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request().times(0);
        mock.expect_answer_callback().times(0);
        mock.expect_edit_message().times(0);
        mock.expect_send_message().times(0);
        let event = mock_event(mock, "m_main");
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
    }

    /// 空 msg_id（无原消息可编辑）时直接 send。
    #[tokio::test]
    async fn empty_msg_id_falls_back_to_send() {
        let mut mock = MockBotAdapter::new();
        mock.expect_accept_contact_request()
            .times(1)
            .returning(|_| Ok(()));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_edit_message().times(0);
        mock.expect_send_message()
            .times(1)
            .returning(|_, _| Ok(MessageId("2".into())));
        let mut event = mock_event(mock, "sx_approve:7");
        event.msg_id = MessageId(String::new());
        assert_eq!(handle(&event).await.unwrap(), HandlerAction::Done);
    }
}
