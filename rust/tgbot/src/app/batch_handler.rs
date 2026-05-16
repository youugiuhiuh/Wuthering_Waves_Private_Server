use std::io::Write;

use teloxide::prelude::*;
use teloxide::types::{ChatId, InputFile, MessageId, ParseMode};
use tempfile::NamedTempFile;
use tokio::time::{Duration, sleep};

use tgbot::core::types::BatchCreationResult;

/// Send SingBox batch creation results to the user:
/// header message, chunked link messages, document file, summary message,
/// then auto-delete all messages after 60 seconds.
///
/// Uses NamedTempFile for automatic cleanup of the temporary document file.
pub async fn send_singbox_batch_result(
    bot: &Bot,
    chat_id: ChatId,
    protocol_name: &str,
    result: &BatchCreationResult,
) -> anyhow::Result<()> {
    let mut message_ids: Vec<MessageId> = Vec::new();

    let header_msg = format!(
        "✅ <b>{} 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
        protocol_name,
        result.created_count,
        result.config_file.as_deref().unwrap_or("未知")
    );
    if let Ok(msg) = bot
        .send_message(chat_id, header_msg)
        .parse_mode(ParseMode::Html)
        .await
    {
        message_ids.push(msg.id);
    }

    let mut combined_links = String::new();
    for (i, link) in result.links.iter().enumerate() {
        combined_links.push_str(&format!("<code>{}</code>\n\n", link));
        if (i + 1) % 2 == 0 {
            if let Ok(msg) = bot
                .send_message(chat_id, combined_links.clone())
                .parse_mode(ParseMode::Html)
                .await
            {
                message_ids.push(msg.id);
            }
            combined_links.clear();
        }
    }
    if !combined_links.is_empty()
        && let Ok(msg) = bot
            .send_message(chat_id, combined_links)
            .parse_mode(ParseMode::Html)
            .await
    {
        message_ids.push(msg.id);
    }

    let links_text = result.links.join("\n");
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(links_text.as_bytes())?;
    temp_file.flush()?;

    let file_path = temp_file.path().to_path_buf();
    if let Ok(msg) = bot
        .send_document(chat_id, InputFile::file(&file_path))
        .caption("完整链接列表，建议尽快复制/导入")
        .await
    {
        message_ids.push(msg.id);
    }

    let result_msg = format!("✅ 批量创建完成！\n\n📊 生成数量: {}", result.created_count);
    if let Ok(msg) = bot.send_message(chat_id, result_msg).await {
        message_ids.push(msg.id);
    }

    let bot_clone = bot.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(60)).await;
        for msg_id in message_ids {
            if let Err(e) = bot_clone.delete_message(chat_id, msg_id).await {
                log::warn!(
                    "删除消息失败 (chat_id: {}, msg_id: {}): {}",
                    chat_id,
                    msg_id,
                    e
                );
            }
        }
    });

    Ok(())
}
