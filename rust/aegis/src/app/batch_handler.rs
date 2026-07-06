use std::sync::Arc;

use teloxide::types::ChatId;
use tokio::time::{Duration, sleep};

use aegis::adapters::common::{BotAdapter, MessageContent, MessageId, TargetId};
use aegis::core::types::BatchCreationResult;

/// Send SingBox batch creation results through the adapter (supports routing):
/// header message, chunked link messages, summary message,
/// then best-effort auto-delete after 60 seconds.
/// Sensitive content (protocol links) is routed through the adapter's routing logic.
pub async fn send_singbox_batch_result(
    adapter: Arc<dyn BotAdapter>,
    chat_id: ChatId,
    protocol_name: &str,
    result: &BatchCreationResult,
) -> anyhow::Result<()> {
    let target = TargetId(chat_id.0.to_string());
    let mut message_ids: Vec<String> = Vec::new();

    let header_msg = format!(
        "✅ <b>{} 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
        protocol_name,
        result.created_count,
        result.config_file.as_deref().unwrap_or("未知")
    );
    if let Ok(msg) = adapter
        .send_message(
            &target,
            MessageContent {
                text: header_msg,
                markup: None,
            },
        )
        .await
    {
        message_ids.push(msg.0);
    }

    let mut combined_links = String::new();
    for link in &result.links {
        combined_links.push_str(link);
        combined_links.push_str("\n\n");
    }
    if !combined_links.is_empty()
        && let Ok(msg) = adapter
            .send_message(
                &target,
                MessageContent {
                    text: combined_links,
                    markup: None,
                },
            )
            .await
    {
        message_ids.push(msg.0);
    }

    let result_msg = format!("✅ 批量创建完成！\n\n📊 生成数量: {}", result.created_count);
    if let Ok(msg) = adapter
        .send_message(
            &target,
            MessageContent {
                text: result_msg,
                markup: None,
            },
        )
        .await
    {
        message_ids.push(msg.0);
    }

    let adapter_clone = adapter.clone();
    let target_clone = target.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(60)).await;
        for id_str in message_ids {
            let mid = MessageId(id_str);
            if let Err(e) = adapter_clone.delete_message(&target_clone, &mid).await {
                log::warn!("删除批量创建消息失败: {}", e);
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::adapters::common::MockBotAdapter;
    use aegis::core::types::BatchCreationResult;

    fn make_result(
        created_count: usize,
        links: Vec<&str>,
        config_file: Option<&str>,
    ) -> BatchCreationResult {
        BatchCreationResult {
            created_count,
            links: links.into_iter().map(String::from).collect(),
            config_file: config_file.map(String::from),
            backup_file: None,
        }
    }

    #[tokio::test]
    async fn sends_header_links_and_result_messages() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(3)
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_delete_message().returning(|_, _| Ok(()));

        let result = make_result(2, vec!["vless://a", "vless://b"], Some("/tmp/cfg.json"));
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn skips_links_when_empty() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(2) // header + result (no links)
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_delete_message().returning(|_, _| Ok(()));

        let result = make_result(0, vec![], None);
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handles_adapter_send_failure_gracefully() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .times(3)
            .returning(|_, _| Err(anyhow::anyhow!("network error")));
        mock.expect_delete_message().returning(|_, _| Ok(()));

        let result = make_result(5, vec!["vless://x"], Some("/tmp/x.json"));
        let output = send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result).await;
        assert!(output.is_ok());
    }

    #[tokio::test]
    async fn includes_protocol_name_in_header() {
        let mut mock = MockBotAdapter::new();
        mock.expect_send_message()
            .withf(|_, content| content.text.contains("hy2"))
            .times(1)
            .returning(|_, _| Ok(MessageId("1".to_string())));
        mock.expect_send_message()
            .returning(|_, _| Ok(MessageId("2".to_string())));
        mock.expect_delete_message().returning(|_, _| Ok(()));

        let result = make_result(1, vec!["vless://x"], Some("/tmp/x.json"));
        send_singbox_batch_result(Arc::new(mock), ChatId(1), "hy2", &result)
            .await
            .unwrap();
    }
}
