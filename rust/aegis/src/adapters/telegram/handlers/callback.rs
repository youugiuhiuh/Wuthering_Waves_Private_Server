use crate::app::destruct_flow;
use crate::app::destruct_flow::MessageFlowOutcome;
use crate::app::state::{AppState, TimeoutStatus};
use crate::save_lang_to_config;
use aegis::core::i18n;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::prelude::{CallbackQuery, ChatId, Requester, ResponseResult};

pub fn handle_callback(
    bot: Bot,
    mut q: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        loop {
            let user_id = q.from.id.0 as i64;
            if !state.is_authorized(user_id).await {
                bot.answer_callback_query(q.id)
                    .text(rust_i18n::t!("auth.expired"))
                    .await?;
                break Ok(());
            }

            let data = match q.data.as_ref() {
                Some(d) => d.clone(),
                None => break Ok(()),
            };
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();

            if data.starts_with("lang:") {
                let lang = match data.as_str() {
                    "lang:zh" => i18n::Lang::Zh,
                    "lang:en" => i18n::Lang::En,
                    "lang:ja" => i18n::Lang::Ja,
                    _ => {
                        bot.answer_callback_query(q.id).await?;
                        break Ok(());
                    }
                };
                i18n::set_lang(lang);
                state.set_lang(lang).await;
                let _ = save_lang_to_config(&state, lang).await;
                state.mark_lang_configured().await;
                i18n::mark_lang_configured();
                let tz = i18n::lang_to_timezone(lang);

                match tokio::process::Command::new("timedatectl")
                    .args(["set-timezone", tz])
                    .output()
                    .await
                {
                    Ok(o) if !o.status.success() => {
                        log::warn!("设置系统时区 {} 失败: exit {:?}", tz, o.status.code());
                    }
                    Err(e) => log::warn!("设置系统时区 {} 失败: {}", tz, e),
                    _ => {}
                }

                if let Err(e) =
                    aegis::core::system::operations::Operations::set_apt_daily_timer().await
                {
                    log::warn!("覆盖 apt-daily timer 失败: {}", e);
                }

                if let Err(e) =
                    aegis::core::system::operations::Operations::perform_maintenance_with_reboot_time(
                        aegis::core::system::operations::Operations::DEFAULT_REBOOT_TIME
                    ).await
                {
                    log::warn!("安全更新初始化失败: {}", e);
                }

                if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
                    let geo_task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
                        aegis::core::system::scheduler::TaskType::GeoUpdate,
                        "0 1 * * 1",
                        tz,
                    );
                    let _ = manager.add_new_task(geo_task).await;
                }
                bot.answer_callback_query(q.id.clone())
                    .text(rust_i18n::t!("lang.switched", "0" => lang.as_str()))
                    .await?;
                let new_q = q.clone();
                q = CallbackQuery {
                    data: Some("m_main".to_string()),
                    ..new_q
                };
                continue;
            }

            if destruct_flow::handle_callback_timeout(&bot, &q, chat_id, msg_id, &state).await?
                == MessageFlowOutcome::Handled
            {
                break Ok(());
            }

            let is_custom_followup = data.starts_with("s_custom_ui:")
                || data.starts_with("s_custom_set:")
                || data == "s_custom_confirm"
                || data == "s_custom_cancel";
            let chat_id_str = chat_id.0.to_string();
            if is_custom_followup
                && state
                    .schedule_timeout_status(&chat_id_str, Duration::from_secs(180))
                    .await
                    == TimeoutStatus::Expired
            {
                state.remove_schedule_input(&chat_id_str).await;
                let new_q = q.clone();
                q = CallbackQuery {
                    data: Some("s_add_custom_menu".to_string()),
                    ..new_q
                };
                bot.answer_callback_query(q.id.clone())
                    .text(rust_i18n::t!("schedule.input_timeout"))
                    .show_alert(true)
                    .await?;
                continue;
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

            let ctx = super::context::CallbackContext {
                bot: bot.clone(),
                q: q.clone(),
                state: state.clone(),
                chat_id,
                msg_id,
                user_id,
                data: data.clone(),
            };

            match super::dispatch(&ctx).await {
                Ok(Some(action)) => match action {
                    super::context::HandlerAction::Done => break Ok(()),
                    super::context::HandlerAction::Redirect(new_data) => {
                        let new_q = q.clone();
                        q = CallbackQuery {
                            data: Some(new_data),
                            ..new_q
                        };
                        continue;
                    }
                },
                Ok(None) => {} // No handler matched
                Err(e) => {
                    eprintln!("[ERROR] Handler dispatch failed: {:?}", e);
                    let _ = bot
                        .answer_callback_query(q.id.clone())
                        .text(rust_i18n::t!("callback.internal_error"))
                        .show_alert(true)
                        .await;
                    break Ok(());
                }
            }

            bot.answer_callback_query(q.id).await?;
            break Ok(());
        }
    })
}
