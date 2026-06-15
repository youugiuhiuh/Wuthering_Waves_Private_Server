use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

use crate::app::state::{AppState, DestructStep, TimeoutStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageFlowOutcome {
    Handled,
    NotHandled,
}

pub async fn handle_message_flow(
    bot: &Bot,
    msg: &Message,
    user_id: i64,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id = msg.chat.id;
    let chat_id_str = chat_id.0.to_string();
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.send_message(chat_id, "⏳ 自毁流程超时 (60s)，已自动取消。")
                .await?;
            return Ok(MessageFlowOutcome::Handled);
        }
        TimeoutStatus::NotTracked => return Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::Active => {}
    }

    if !state.is_authorized(user_id).await {
        bot.send_message(chat_id, "⚠️ 会话已过期，请重新认证")
            .await?;
        return Ok(MessageFlowOutcome::Handled);
    }

    let Some(destruct_state) = state.destruct_snapshot(&chat_id_str).await else {
        return Ok(MessageFlowOutcome::NotHandled);
    };

    match destruct_state.step {
        DestructStep::AwaitFirstTotp => {
            if let Some(text) = msg.text() {
                let text = text.trim();
                if state.verify_totp(text) {
                    if state
                        .confirm_first_destruct_totp(&chat_id_str, text, Instant::now())
                        .await
                    {
                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::callback(
                                "⚠️ 确认执行销毁",
                                "a_destroy_confirm",
                            )],
                            vec![InlineKeyboardButton::callback(
                                "🔙 取消",
                                "a_destroy_cancel",
                            )],
                        ]);
                        bot.send_message(
                            chat_id,
                            "⚠️ <b>危险操作确认 (2/4)</b>\n\n验证通过。\n请点击下方按钮确认执行销毁。\n此操作<b>不可逆</b>！",
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                    }
                } else {
                    bot.send_message(chat_id, "❌ 验证码错误，请重新输入。")
                        .await?;
                }
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitSecondTotp => {
            if let Some(text) = msg.text() {
                let text = text.trim();
                if state.verify_totp(text) {
                    match state
                        .confirm_second_destruct_totp(&chat_id_str, text, Instant::now())
                        .await
                    {
                        Err(_) => {
                            bot.send_message(chat_id, "❌ <b>安全警告 (防重放)</b>\n\n为了防止重放攻击，请等待下一个 TOTP 验证码（30秒刷新）。\n不能使用与第一次相同的验证码。").parse_mode(ParseMode::Html).await?;
                        }
                        Ok(true) => {
                            bot.send_message(
                                chat_id,
                                "🚨 <b>最终验证 (4/4)</b>\n\n请输入<b>安全验证文件</b> (图片或文档)。\n系统将比对文件指纹 (SHA-256) 以授权最终销毁。\n\n(如果没有设置安全文件，请使用 /set_security_file 先设置)",
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                        }
                        Ok(false) => {
                            bot.send_message(chat_id, "状态无效，请重新开始").await?;
                        }
                    }
                } else {
                    bot.send_message(chat_id, "❌ 验证码错误，请重新输入。")
                        .await?;
                }
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitSecurityFile => {
            let (file_id, file_name) = if let Some(doc) = msg.document() {
                (Some(doc.file.id.clone()), doc.file_name.clone())
            } else if let Some(photos) = msg.photo() {
                (
                    photos.last().map(|p| p.file.id.clone()),
                    Some("图片".to_string()),
                )
            } else {
                (None, None)
            };

            if let Some(fid) = file_id {
                let file = bot.get_file(fid.clone()).await?;
                let mut content = Vec::new();
                bot.download_file(&file.path, &mut content)
                    .await
                    .map_err(std::io::Error::other)?;

                let mut hasher = Sha256::new();
                hasher.update(&content);
                let hash_hex = hex::encode(hasher.finalize());

                if let Some(correct) = state.self_destruct_key_hash().await {
                    if hash_hex == correct {
                        let hash_short = if hash_hex.len() > 12 {
                            format!("{}...{}", &hash_hex[..8], &hash_hex[hash_hex.len() - 4..])
                        } else {
                            hash_hex.clone()
                        };
                        let file_display = file_name
                            .map(|n| format!("{} | {}", n, hash_short))
                            .unwrap_or_else(|| hash_short.clone());

                        if state
                            .mark_destruct_file_verified(&chat_id_str, Instant::now())
                            .await
                        {
                            let keyboard = InlineKeyboardMarkup::new(vec![
                                vec![InlineKeyboardButton::callback(
                                    "💀 最终确认销毁 (BOOM)",
                                    "a_destroy_final",
                                )],
                                vec![InlineKeyboardButton::callback(
                                    "🔙 取消",
                                    "a_destroy_cancel",
                                )],
                            ]);

                            bot.send_message(
                                chat_id,
                                format!(
                                    "☠️ <b>授权通过</b>\n\n指纹匹配成功 ({})。\n这是最后的确认，点击后服务器将<b>永久变砖</b>。",
                                    file_display
                                ),
                            )
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard)
                            .await?;
                        }
                    } else {
                        bot.send_message(chat_id, "❌ 文件验证失败。\nHash 不匹配。")
                            .await?;
                    }
                } else {
                    bot.send_message(chat_id, "❌ 系统未设置安全验证文件，无法执行销毁。\n请先取消流程并使用 /set_security_file 设置文件。").await?;
                }
            } else {
                bot.send_message(chat_id, "⚠️ 请发送安全验证文件 (图片或文档)。")
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        DestructStep::AwaitConfirm | DestructStep::AwaitFinalConfirm => {
            Ok(MessageFlowOutcome::Handled)
        }
    }
}

pub async fn handle_callback_timeout(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id_str = chat_id.0.to_string();
    match state
        .touch_destruct(&chat_id_str, Instant::now(), Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_destruct(&chat_id_str).await;
            bot.answer_callback_query(q.id.clone())
                .text("⏳ 流程已超时 (60s)")
                .await?;
            bot.edit_message_text(chat_id, msg_id, "⏳ 自毁流程超时 (60s)，已自动取消。")
                .parse_mode(ParseMode::Html)
                .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        TimeoutStatus::Active => Ok(MessageFlowOutcome::NotHandled),
        TimeoutStatus::NotTracked => Ok(MessageFlowOutcome::NotHandled),
    }
}

pub async fn handle_callback_action(
    bot: &Bot,
    q: &CallbackQuery,
    data: &str,
    chat_id: ChatId,
    msg_id: teloxide::types::MessageId,
    state: &Arc<AppState>,
) -> ResponseResult<MessageFlowOutcome> {
    let chat_id_str = chat_id.0.to_string();
    match data {
        "a_destroy_ask" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text("⚠️ 会话已过期，请重新认证")
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            state.begin_destruct(chat_id_str.clone(), Instant::now()).await;
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                "🔙 取消",
                "a_destroy_cancel",
            )]]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                "⚠️ <b>危险操作确认 (1/3)</b>\n\n您正在请求执行<b>自毁程序 (焦土模式)</b>。\n此操作将<b>递归删除服务器根目录下所有文件 (rm -rf /)</b>，且<b>不可恢复</b>。\n\n请输入 TOTP 验证码以继续:",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_cancel" => {
            if state.cancel_destruct(&chat_id_str).await {
                bot.send_message(chat_id, "操作已取消。").await?;
            }
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "💥 立即自毁",
                    "a_destroy_ask",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
            ]);
            bot.edit_message_text(
                chat_id,
                msg_id,
                "⚠️ <b>危险区域</b>\n\n此处包含不可逆的破坏性操作。\n请谨慎操作！",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_confirm" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text("⚠️ 会话已过期，请重新认证")
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }
            if state
                .advance_destruct_step(
                    &chat_id_str,
                    DestructStep::AwaitConfirm,
                    DestructStep::AwaitSecondTotp,
                    Instant::now(),
                )
                .await
            {
                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        "🔙 取消",
                        "a_destroy_cancel",
                    )]]);
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "⚠️ <b>最终警告 (3/4)</b>\n\n请<b>再次输入新的 TOTP 验证码</b>以确认执行。\n(注意：必须与上一次验证码不同)",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("状态无效，请重新开始")
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        "a_destroy_final" => {
            if !state.is_authorized(chat_id.0).await {
                bot.answer_callback_query(q.id.clone())
                    .text("⚠️ 会话已过期，请重新认证")
                    .await?;
                return Ok(MessageFlowOutcome::Handled);
            }

            let snapshot = state.destruct_snapshot(&chat_id_str).await;
            if snapshot.map(|s| s.step) == Some(DestructStep::AwaitFinalConfirm) {
                bot.answer_callback_query(q.id.clone())
                    .text("正在执行销毁...")
                    .await?;
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    "🚀 <b>最终验证通过。正在执行自毁程序...</b>\n\n所有数据将被擦除，Bot 将停止运行。\n再见。",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                let executor = state.self_destruct_executor();
                aegis::core::security::self_destruct::trigger(executor);
                state.cancel_destruct(&chat_id_str).await;
            } else {
                bot.answer_callback_query(q.id.clone())
                    .text("状态无效")
                    .await?;
            }
            Ok(MessageFlowOutcome::Handled)
        }
        _ => Ok(MessageFlowOutcome::NotHandled),
    }
}
