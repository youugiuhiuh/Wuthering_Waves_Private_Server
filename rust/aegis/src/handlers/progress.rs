use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use crate::adapters::common::{BotAdapter, MessageContent, MessageId, TargetId};

pub fn spawn_progress_updater(
    adapter: Arc<dyn BotAdapter>,
    target: TargetId,
    msg_id: MessageId,
    title_fn: impl Fn(String) -> String + Send + 'static,
) -> (UnboundedSender<String>, JoinHandle<()>) {
    let (tx, mut rx) = unbounded_channel::<String>();
    let handle = tokio::spawn(async move {
        let mut last = String::new();
        while let Some(text) = rx.recv().await {
            if text == last {
                continue;
            }
            last = text.clone();
            let _ = adapter
                .edit_message(
                    &target,
                    &msg_id,
                    MessageContent {
                        text: title_fn(text),
                        markup: None,
                    },
                )
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    (tx, handle)
}
