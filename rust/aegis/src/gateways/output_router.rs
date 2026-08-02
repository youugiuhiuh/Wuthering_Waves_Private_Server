use crate::app::interaction::{ConversationId, OutputAction, Sensitivity};
use crate::app::output::BusinessOutput;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

const PROTECTED_MATRIX_FAILURE_TEXT: &str =
    "Protected content could not be delivered to Matrix. Please try again later.";

pub struct SensitiveOutputRouter {
    origin_output: Arc<dyn BusinessOutput>,
    matrix_output: Option<Arc<dyn BusinessOutput>>,
}

impl SensitiveOutputRouter {
    pub fn new(
        origin_output: Arc<dyn BusinessOutput>,
        matrix_output: Option<Arc<dyn BusinessOutput>>,
    ) -> Self {
        Self {
            origin_output,
            matrix_output,
        }
    }

    pub async fn route(&self, action: OutputAction) -> anyhow::Result<()> {
        match &action {
            OutputAction::SendText {
                sensitivity: Sensitivity::Protected,
                ..
            } => {
                let matrix = self.matrix_output.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("no matrix output configured for protected content")
                })?;
                if let Err(original_err) = matrix.publish(action).await {
                    self.origin_output
                        .publish(OutputAction::SendText {
                            target_conversation: ConversationId::new("system".into()).unwrap(),
                            payload: crate::app::interaction::OutputPayload::Text {
                                text: PROTECTED_MATRIX_FAILURE_TEXT.into(),
                            },
                            sensitivity: Sensitivity::Public,
                        })
                        .await?;
                    return Err(original_err);
                }
                Ok(())
            }
            _ => self.origin_output.publish(action).await,
        }
    }
}

#[derive(Default)]
pub struct FakeOutput {
    pub actions: Arc<Mutex<Vec<OutputAction>>>,
}

impl FakeOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn actions(&self) -> Vec<OutputAction> {
        self.actions.lock().unwrap().clone()
    }

    pub fn contains_text(&self, needle: &str) -> bool {
        self.actions().iter().any(|a| match a {
            OutputAction::SendText {
                payload: crate::app::interaction::OutputPayload::Text { text },
                ..
            } => text.contains(needle),
            _ => false,
        })
    }

    pub fn has_protected_content(&self) -> bool {
        self.actions().iter().any(|a| match a {
            OutputAction::SendText {
                sensitivity: Sensitivity::Protected,
                payload: crate::app::interaction::OutputPayload::Attachment { bytes, .. },
                ..
            } => !bytes.is_empty(),
            _ => false,
        })
    }
}

#[async_trait]
impl BusinessOutput for FakeOutput {
    async fn publish(&self, action: OutputAction) -> anyhow::Result<()> {
        self.actions.lock().unwrap().push(action);
        Ok(())
    }
}

pub struct FailingOutput {
    msg: String,
}

impl FailingOutput {
    pub fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

#[async_trait]
impl BusinessOutput for FailingOutput {
    async fn publish(&self, _action: OutputAction) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(self.msg.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_protected_text_action() -> OutputAction {
        OutputAction::SendText {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            payload: crate::app::interaction::OutputPayload::Text {
                text: "super secret".into(),
            },
            sensitivity: Sensitivity::Protected,
        }
    }

    fn make_protected_attachment_action() -> OutputAction {
        OutputAction::SendText {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            payload: crate::app::interaction::OutputPayload::Attachment {
                bytes: vec![0x00, 0xFF, 0x42],
                filename: "secret.pdf".into(),
                mime: "application/pdf".into(),
            },
            sensitivity: Sensitivity::Protected,
        }
    }

    fn make_public_action() -> OutputAction {
        OutputAction::SendText {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            payload: crate::app::interaction::OutputPayload::Text {
                text: "hello world".into(),
            },
            sensitivity: Sensitivity::Public,
        }
    }

    fn make_edit_action() -> OutputAction {
        OutputAction::Edit {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            message_id: "msg-42".into(),
            payload: crate::app::interaction::OutputPayload::Text {
                text: "updated text".into(),
            },
        }
    }

    fn make_delete_action() -> OutputAction {
        OutputAction::Delete {
            target_conversation: ConversationId::new("chat-1".into()).unwrap(),
            message_id: "msg-42".into(),
        }
    }

    fn make_answer_callback_action() -> OutputAction {
        OutputAction::AnswerCallback {
            callback_id: "cb-99".into(),
            text: Some("done".into()),
        }
    }

    #[tokio::test]
    async fn public_action_routes_to_origin() {
        let origin = Arc::new(FakeOutput::new());
        let matrix = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix));

        router.route(make_public_action()).await.unwrap();

        assert_eq!(origin.actions().len(), 1);
    }

    #[tokio::test]
    async fn single_platform_protected_returns_error() {
        let origin = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), None);

        let err = router
            .route(make_protected_text_action())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no matrix output configured"));
        assert_eq!(origin.actions().len(), 0);
    }

    #[tokio::test]
    async fn protected_action_routes_to_matrix() {
        let origin = Arc::new(FakeOutput::new());
        let matrix = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix.clone()));

        router.route(make_protected_text_action()).await.unwrap();

        assert_eq!(origin.actions().len(), 0);
        assert_eq!(matrix.actions().len(), 1);
    }

    #[tokio::test]
    async fn matrix_failure_publishes_notice_to_origin_and_returns_error() {
        let origin = Arc::new(FakeOutput::new());
        let matrix: Arc<dyn BusinessOutput> = Arc::new(FailingOutput::new("matrix is down"));
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix));

        let err = router
            .route(make_protected_text_action())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("matrix is down"));

        assert_eq!(origin.actions().len(), 1);
        assert!(origin.contains_text(PROTECTED_MATRIX_FAILURE_TEXT));
    }

    #[tokio::test]
    async fn matrix_failure_protected_bytes_never_observed_by_origin() {
        let origin = Arc::new(FakeOutput::new());
        let matrix: Arc<dyn BusinessOutput> = Arc::new(FailingOutput::new("matrix is down"));
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix));

        router
            .route(make_protected_attachment_action())
            .await
            .unwrap_err();

        assert!(
            !origin.has_protected_content(),
            "protected payload bytes must never be observed by origin"
        );
        assert!(origin.contains_text(PROTECTED_MATRIX_FAILURE_TEXT));
    }

    #[tokio::test]
    async fn edit_action_routes_to_origin() {
        let origin = Arc::new(FakeOutput::new());
        let matrix = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix.clone()));

        router.route(make_edit_action()).await.unwrap();

        assert_eq!(origin.actions().len(), 1);
        assert_eq!(matrix.actions().len(), 0);
    }

    #[tokio::test]
    async fn delete_action_routes_to_origin() {
        let origin = Arc::new(FakeOutput::new());
        let matrix = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix.clone()));

        router.route(make_delete_action()).await.unwrap();

        assert_eq!(origin.actions().len(), 1);
        assert_eq!(matrix.actions().len(), 0);
    }

    #[tokio::test]
    async fn answer_callback_routes_to_origin() {
        let origin = Arc::new(FakeOutput::new());
        let matrix = Arc::new(FakeOutput::new());
        let router = SensitiveOutputRouter::new(origin.clone(), Some(matrix.clone()));

        router.route(make_answer_callback_action()).await.unwrap();

        assert_eq!(origin.actions().len(), 1);
        assert_eq!(matrix.actions().len(), 0);
    }
}
