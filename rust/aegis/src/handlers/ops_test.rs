use std::sync::Arc;

use crate::adapters::common::{MessageId, MockBotAdapter, TargetId};
use crate::app::state::AppState;
use crate::core::security::self_destruct::SelfDestructExecutor;
use crate::core::totp::TotpManager;
use futures_util::future::BoxFuture;
use secrecy::SecretString;

use super::context::{HandlerAction, HandlerContext};
use super::progress::spawn_progress_updater;

fn make_state() -> Arc<AppState> {
    Arc::new(AppState::new(
        42,
        TotpManager::new(&SecretString::from(TotpManager::generate_new_secret())).unwrap(),
        Arc::new(NoopExecutor),
        None,
        600,
        Arc::new(MockBotAdapter::new()),
        None,
    ))
}

struct NoopExecutor;
impl SelfDestructExecutor for NoopExecutor {
    fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn spawn_progress_updater_sends_message() {
    let mut mock = MockBotAdapter::new();
    mock.expect_edit_message()
        .withf(|target, msg_id, content| {
            target.0 == "123" && msg_id.0 == "1" && content.text == "prefix: hello"
        })
        .times(1)
        .returning(|_, _, _| Ok(()));

    let adapter = Arc::new(mock);
    let (tx, handle) = spawn_progress_updater(
        adapter,
        TargetId("123".to_string()),
        MessageId("1".to_string()),
        |t| format!("prefix: {}", t),
    );

    tx.send("hello".to_string()).unwrap();
    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn spawn_progress_updater_skips_duplicates() {
    let mut mock = MockBotAdapter::new();
    mock.expect_edit_message()
        .withf(|target, msg_id, content| {
            target.0 == "123" && msg_id.0 == "1" && content.text == "prefix: first"
        })
        .times(1)
        .returning(|_, _, _| Ok(()));

    let adapter = Arc::new(mock);
    let (tx, handle) = spawn_progress_updater(
        adapter,
        TargetId("123".to_string()),
        MessageId("1".to_string()),
        |t| format!("prefix: {}", t),
    );

    tx.send("first".to_string()).unwrap();
    tx.send("first".to_string()).unwrap(); // duplicate - should be skipped
    drop(tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn ops_handle_routes_bbr3_prompt() {
    let mut mock = MockBotAdapter::new();
    mock.expect_edit_message()
        .withf(|_, _, _| true)
        .times(1)
        .returning(|_, _, _| Ok(()));

    let state = make_state();
    let ctx = HandlerContext {
        adapter: &mock,
        target: TargetId("123".to_string()),
        state: &state,
        user_id: 42,
        data: "a_bbr3".to_string(),
        msg_id: Some(MessageId("1".to_string())),
    };
    let result = super::ops::handle(&ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn ops_handle_bbr3_prompt_returns_markup() {
    let mut mock = MockBotAdapter::new();
    mock.expect_edit_message()
        .withf(|target, msg_id, content| {
            target.0 == "123" && msg_id.0 == "1" && content.markup.is_some()
        })
        .times(1)
        .returning(|_, _, _| Ok(()));

    let state = make_state();
    let ctx = HandlerContext {
        adapter: &mock,
        target: TargetId("123".to_string()),
        state: &state,
        user_id: 42,
        data: "a_bbr3".to_string(),
        msg_id: Some(MessageId("1".to_string())),
    };
    let result = super::ops::handle(&ctx).await;
    assert!(matches!(result.unwrap(), HandlerAction::Done));
}

#[tokio::test]
async fn ops_handle_bbr3_cancel_returns_redirect() {
    let mock = MockBotAdapter::new();
    let state = make_state();
    let ctx = HandlerContext {
        adapter: &mock,
        target: TargetId("123".to_string()),
        state: &state,
        user_id: 42,
        data: "a_bbr3_cancel".to_string(),
        msg_id: Some(MessageId("1".to_string())),
    };
    let result = super::ops::handle(&ctx).await.unwrap();
    assert!(matches!(result, HandlerAction::Redirect(_)));
}

#[tokio::test]
async fn ops_handle_unknown_data_returns_done() {
    let mock = MockBotAdapter::new();
    let state = make_state();
    let ctx = HandlerContext {
        adapter: &mock,
        target: TargetId("123".to_string()),
        state: &state,
        user_id: 42,
        data: "unknown_data".to_string(),
        msg_id: None,
    };
    let result = super::ops::handle(&ctx).await;
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), HandlerAction::Done));
}

#[tokio::test]
async fn ops_handle_reload_returns_done() {
    let mut mock = MockBotAdapter::new();
    mock.expect_send_message()
        .withf(|_, _| true)
        .times(1)
        .returning(|_, _| Ok(MessageId("2".to_string())));

    let state = make_state();
    let ctx = HandlerContext {
        adapter: &mock,
        target: TargetId("123".to_string()),
        state: &state,
        user_id: 42,
        data: "a_reload".to_string(),
        msg_id: None,
    };
    let result = super::ops::handle(&ctx).await;
    assert!(result.is_ok());
}
