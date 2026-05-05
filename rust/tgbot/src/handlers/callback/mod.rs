//! 回调查询处理模块
//!
//! 根据 callback data 前缀分派到对应的处理器:
//! - `sb_*` → SingBox 配置管理
//! - `s_*` / `s_custom_*` → 定时任务管理
//! - `cfg_*` → 配置删除管理
//! - `a_warp_*` → WARP 分流管理
//! - `u_*` → Xray 核心配置管理

pub mod singbox;
pub mod schedule;
pub mod config;
pub mod warp;
pub mod xraycore;

use teloxide::prelude::*;
use teloxide::types::CallbackQuery;
use std::sync::Arc;

use crate::app::state::AppState;
use crate::app::destruct_flow::{self, MessageFlowOutcome};

/// 处理回调查询分派
///
/// 根据 callback data 前缀分派到对应的处理器.
/// 支持超时检测、定时任务输入流、WARP 输入流、销毁流程等.
///
/// # Arguments
/// * `bot` - Telegram bot 实例
/// * `q` - 回调查询
/// * `state` - 应用状态
///
/// # Returns
/// 处理结果
pub async fn handle_callback(
    bot: Bot,
    mut q: CallbackQuery,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    loop {
        let user_id = q.from.id.0 as i64;
        if !state.is_authorized(user_id).await {
            bot.answer_callback_query(q.id)
                .text("🚫 会话已过期，请发送 6 位 TOTP 验证码重新认证")
                .await?;
            break Ok(());
        }

        let data = match q.data.as_ref() {
            Some(d) => d.clone(),
            None => break Ok(()),
        };
        let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
        let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

        if destruct_flow::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
            == MessageFlowOutcome::Handled
        {
            break Ok(());
        }

        let is_custom_followup = data.starts_with("s_custom_ui:")
            || data.starts_with("s_custom_set:")
            || data == "s_custom_confirm"
            || data == "s_custom_cancel";
        if is_custom_followup {
            if state
                .schedule_timeout_status(chat_id, std::time::Duration::from_secs(180))
                .await
                == crate::app::state::TimeoutStatus::Expired
            {
                state.remove_schedule_input(chat_id).await;
                let new_q = q.clone();
                q = CallbackQuery {
                    data: Some("s_add_custom_menu".to_string()),
                    ..new_q
                };
                bot.answer_callback_query(q.id.clone())
                    .text("⏳ 自定义定时会话已超时，请重新进入。")
                    .show_alert(true)
                    .await?;
                continue;
            }
        }

        if destruct_flow::handle_callback_action(
            &bot,
            &q,
            data.as_str(),
            chat_id,
            msg_id,
            &state,
        )
        .await?
            == MessageFlowOutcome::Handled
        {
            break Ok(());
        }

        if data.starts_with("sb_") {
            singbox::dispatch_callback(&bot, &q, chat_id, msg_id, &data, &state).await?;
            break Ok(());
        }

        if data.starts_with("s_") || data.starts_with("s_custom") {
            schedule::dispatch_callback(&bot, &q, chat_id, msg_id, &data, &state).await?;
            break Ok(());
        }

        if data.starts_with("cfg_") {
            config::dispatch_callback(&bot, &q, chat_id, msg_id, &data, &state).await?;
            break Ok(());
        }

        if data.starts_with("a_warp_") {
            warp::dispatch_callback(&bot, &q, chat_id, msg_id, &data, &state).await?;
            break Ok(());
        }

        if data.starts_with("u_") {
            xraycore::handle_xraycore_callback(bot, q, chat_id, msg_id, &data, state).await?;
            break Ok(());
        }

        break Ok(());
    }
}
