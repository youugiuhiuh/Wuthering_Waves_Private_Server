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
    for (i, link) in result.links.iter().enumerate() {
        combined_links.push_str(&format!("<code>{}</code>\n\n", link));
        if (i + 1) % 2 == 0 {
            if let Ok(msg) = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: combined_links.clone(),
                        markup: None,
                    },
                )
                .await
            {
                message_ids.push(msg.0);
            }
            combined_links.clear();
        }
    }
    if !combined_links.is_empty() {
        if let Ok(msg) = adapter
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
