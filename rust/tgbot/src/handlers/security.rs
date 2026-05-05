use crate::logic::maintenance::MaintenanceManager;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::mpsc;

pub async fn handle_firewall_harden(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
) -> ResponseResult<()> {
    let q_id = q.id.clone();
    let bot_clone = bot.clone();

    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        let bot_for_updates = bot_clone.clone();
        let update_task = tokio::spawn(async move {
            let mut last_text = String::new();
            while let Some(text) = rx.recv().await {
                if text == last_text {
                    continue;
                }
                last_text = text.clone();
                let _ = bot_for_updates
                    .edit_message_text(
                        chat_id,
                        msg_id,
                        format!("🛡️ <b>防火墙安全加固</b>\n{}", text),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });

        let tx_clone = tx.clone();
        let res_timeout = tokio::time::timeout(
            Duration::from_secs(45),
            MaintenanceManager::harden_firewall(move |text| {
                let _ = tx_clone.send(text.to_string());
            }),
        )
        .await;

        match res_timeout {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                let _ = tx.send(format!("❌ 失败: {}", err));
            }
            Err(_) => {
                let _ = tx.send(
                    "❌ 失败: 操作超时 (45s)，请检查系统 nftables 状态".to_string(),
                );
            }
        }

        drop(tx);
        let _ = update_task.await;
    });

    bot.answer_callback_query(q_id)
        .text("⚙️ 正在启动防火墙扫描与加固...")
        .await?;

    Ok(())
}
