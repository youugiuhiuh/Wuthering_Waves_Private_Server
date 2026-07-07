use std::sync::Arc;

use anyhow::Result;

use crate::adapters::common::{BotAdapter, Markup, MessageId, TargetId};
use crate::app::state::AppState;

pub struct HandlerContext<'a> {
    pub adapter: &'a dyn BotAdapter,
    pub target: TargetId,
    pub state: &'a Arc<AppState>,
    pub user_id: i64,
    pub data: String,
    pub msg_id: Option<MessageId>,
}

pub enum HandlerAction {
    Done,
    Redirect(String),
}

pub type HandlerResult = Result<HandlerAction>;

impl HandlerContext<'_> {
    pub async fn reply(&self, text: String) -> Result<MessageId> {
        self.adapter
            .send_message(
                &self.target,
                crate::adapters::common::MessageContent { text, markup: None },
            )
            .await
    }

    pub async fn reply_markup(&self, text: String, markup: Markup) -> Result<MessageId> {
        self.adapter
            .send_message(
                &self.target,
                crate::adapters::common::MessageContent {
                    text,
                    markup: Some(markup),
                },
            )
            .await
    }

    pub async fn edit(&self, text: String) -> Result<()> {
        if let Some(msg_id) = &self.msg_id {
            self.adapter
                .edit_message(
                    &self.target,
                    msg_id,
                    crate::adapters::common::MessageContent { text, markup: None },
                )
                .await?;
        }
        Ok(())
    }

    pub async fn edit_markup(&self, text: String, markup: Markup) -> Result<()> {
        if let Some(msg_id) = &self.msg_id {
            self.adapter
                .edit_message(
                    &self.target,
                    msg_id,
                    crate::adapters::common::MessageContent {
                        text,
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        Ok(())
    }

    pub async fn delete(&self) -> Result<()> {
        if let Some(msg_id) = &self.msg_id {
            self.adapter.delete_message(&self.target, msg_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::MockBotAdapter;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::core::totp::TotpManager;
    use futures_util::future::BoxFuture;
    use secrecy::SecretString;

    struct NoopExecutor;
    impl SelfDestructExecutor for NoopExecutor {
        fn execute(&self) -> BoxFuture<'static, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

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

    #[tokio::test]
    async fn handler_context_reply_sends_message() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .withf(|target, content| target.0 == "123" && content.text == "hello")
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));

        let state = make_state();
        let ctx = HandlerContext {
            adapter: &mock,
            target: TargetId("123".to_string()),
            state: &state,
            user_id: 42,
            data: "test".to_string(),
            msg_id: None,
        };
        let result = ctx.reply("hello".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handler_context_edit_requires_msg_id() {
        let mock = MockBotAdapter::new();
        let state = make_state();
        let ctx = HandlerContext {
            adapter: &mock,
            target: TargetId("123".to_string()),
            state: &state,
            user_id: 42,
            data: "test".to_string(),
            msg_id: None,
        };
        // Without msg_id, edit should be no-op
        let result = ctx.edit("hello".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handler_context_edit_with_msg_id() {
        let mut mock = MockBotAdapter::new();
        mock.expect_edit_message()
            .withf(|target, msg_id, content| {
                target.0 == "123" && msg_id.0 == "5" && content.text == "updated"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));

        let state = make_state();
        let ctx = HandlerContext {
            adapter: &mock,
            target: TargetId("123".to_string()),
            state: &state,
            user_id: 42,
            data: "test".to_string(),
            msg_id: Some(MessageId("5".to_string())),
        };
        let result = ctx.edit("updated".to_string()).await;
        assert!(result.is_ok());
    }
}
