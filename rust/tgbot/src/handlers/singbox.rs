use super::context::{CallbackContext, HandlerAction, HandlerResult};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let bot = &ctx.bot;
    let chat_id = ctx.chat_id;
    let msg_id = ctx.msg_id;
    let q = &ctx.q;

    match ctx.data.as_str() {
        "m_singbox_mgmt" => {
            let is_installed = SingBoxInstaller::is_installed().await;
            let inbounds = SingBoxConfigManager::list_all_inbound_files()
                .await
                .unwrap_or_default();
            let mut buttons = Vec::new();

            if !is_installed {
                buttons.push(vec![InlineKeyboardButton::callback(
                    "🚀 安装 Sing-box",
                    "sb_install",
                )]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未检测到 Sing-box</b>\n\n系统尚未安装 Sing-box，无法使用 Hysteria2/TUIC 协议。",
                )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else if inbounds.is_empty() {
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                    InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "📦 <b>Sing-box 管理</b>\n\n⚠️ <b>未找到配置文件</b>\n\n您可以创建 Hysteria2 或 TUIC 批量配置。",
                )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else {
                for (i, path) in inbounds.iter().enumerate() {
                    let filename = path.split('/').next_back().unwrap_or("Unknown");
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("📁 {}", filename),
                        format!("sb_l:{}", i),
                    )]);
                }
                buttons.push(vec![InlineKeyboardButton::callback(
                    "🗑️ 删除管理",
                    "sb_del_cfg",
                )]);
                buttons.push(vec![
                    InlineKeyboardButton::callback("🚀 Hysteria2 批量", "sb_h2_init"),
                    InlineKeyboardButton::callback("🚀 TUIC 批量", "sb_tu_init"),
                ]);
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "📦 <b>Sing-box 管理</b>\n选择配置文件:",
                )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }

            // 每一个独立分支处理完，显式返回 Done 告诉闸机我们处理完了
            Ok(HandlerAction::Done)
        }

        "sb_install" => {
            bot.answer_callback_query(q.id.clone())
                .text("⏳ 正在安装 Sing-box...")
                .await?;

            // 💡 克隆一份 Bot 传进跨线程的 tokio::spawn，避免生命周期报错
            let bot_clone = bot.clone();
            tokio::spawn(async move {
                match SingBoxInstaller::install().await {
                    Ok(_) => {
                        let _ = bot_clone.send_message(chat_id, "✅ <b>Sing-box 安装成功！</b>\n\n现在您可以创建 Hysteria2 或 TUIC 配置了。").parse_mode(ParseMode::Html).await;
                    }
                    Err(e) => {
                        let _ = bot_clone
                            .send_message(
                                chat_id,
                                format!("❌ <b>安装失败</b>\n原因: {}", e),
                            )
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
            });

            Ok(HandlerAction::Done)
        }

        // 兜底分支：如果新模块收到无法识别的点击事件，同样安全返回
        _ => Ok(HandlerAction::Done),
    }
}