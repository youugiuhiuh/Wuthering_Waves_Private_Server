use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use super::keyboard::{
    build_cron_from_custom_state, build_custom_day_keyboard, build_custom_hour_keyboard,
    build_custom_minute_keyboard, build_custom_schedule_keyboard, build_custom_schedule_text,
    build_custom_timezone_keyboard,
};
use crate::utils;
use aegis::app::state::{ScheduleFrequency, ScheduleInputState};
use aegis::core::system::operations::Operations;
use aegis::core::system::scheduler::TaskType;
use rust_i18n::t;
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::*;

pub(super) async fn handle_sched(ctx: &CallbackContext) -> HandlerResult {
    ctx.state
        .remove_schedule_input(&ctx.chat_id.0.to_string())
        .await;
    let summary = if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        manager.get_summary().await
    } else {
        t!("schedule.scheduler_not_init").to_string()
    };

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("schedule.add_task"), "s_add_menu"),
            InlineKeyboardButton::callback(t!("schedule.del_task"), "s_del_menu"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back_settings"),
            "m_settings",
        )],
    ]);

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, summary)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_add_menu(ctx: &CallbackContext) -> HandlerResult {
    ctx.state
        .remove_schedule_input(&ctx.chat_id.0.to_string())
        .await;
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            t!("schedule.daily_4am"),
            "s_add:reload_daily_4",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_btn"),
            "s_add_custom_menu",
        )],
        vec![InlineKeyboardButton::callback(t!("menu.back"), "m_sched")],
    ]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.add_menu_title"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_add_custom_menu(ctx: &CallbackContext) -> HandlerResult {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_geo"),
                    t!("schedule.freq_daily")
                ),
                "s_custom:geo:daily",
            ),
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_geo"),
                    t!("schedule.freq_weekly")
                ),
                "s_custom:geo:weekly",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_reload"),
                    t!("schedule.freq_daily")
                ),
                "s_custom:reload:daily",
            ),
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_reload"),
                    t!("schedule.freq_weekly")
                ),
                "s_custom:reload:weekly",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_reboot"),
                    t!("schedule.freq_daily")
                ),
                "s_custom:reboot:daily",
            ),
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_reboot"),
                    t!("schedule.freq_weekly")
                ),
                "s_custom:reboot:weekly",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_security"),
                    t!("schedule.freq_daily")
                ),
                "s_custom:secupd:daily",
            ),
            InlineKeyboardButton::callback(
                format!(
                    "{} - {}",
                    t!("schedule.task_type_security"),
                    t!("schedule.freq_weekly")
                ),
                "s_custom:secupd:weekly",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "s_add_menu",
        )],
    ]);
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.custom_title"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_new(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let mut parts = data.split(':');
    let _prefix = parts.next();
    let task_part = parts.next();
    let freq_part = parts.next();

    let (task_type, frequency) = match (task_part, freq_part) {
        (Some("geo"), Some("daily")) => (TaskType::GeoUpdate, ScheduleFrequency::Daily),
        (Some("geo"), Some("weekly")) => (TaskType::GeoUpdate, ScheduleFrequency::Weekly),
        (Some("reload"), Some("daily")) => (TaskType::ReloadCore, ScheduleFrequency::Daily),
        (Some("reload"), Some("weekly")) => (TaskType::ReloadCore, ScheduleFrequency::Weekly),
        (Some("reboot"), Some("daily")) => (TaskType::Reboot, ScheduleFrequency::Daily),
        (Some("reboot"), Some("weekly")) => (TaskType::Reboot, ScheduleFrequency::Weekly),
        (Some("secupd"), Some("daily")) => (TaskType::SecurityUpdate, ScheduleFrequency::Daily),
        (Some("secupd"), Some("weekly")) => (TaskType::SecurityUpdate, ScheduleFrequency::Weekly),
        _ => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("schedule.invalid_template"))
                .await?;
            return Ok(HandlerAction::Done);
        }
    };

    let return_to = match &task_type {
        TaskType::GeoUpdate => "a_geo_sched_menu",
        _ => "s_add_custom_menu",
    };
    ctx.state
        .insert_schedule_input(
            ctx.chat_id.0.to_string(),
            ScheduleInputState {
                updated_at: Instant::now(),
                task_type: task_type.clone(),
                frequency,
                timezone: "UTC".to_string(),
                day_of_week: None,
                hour: None,
                minute: None,
                return_to: return_to.to_string(),
            },
        )
        .await;

    let Some(input_state) = ctx
        .state
        .schedule_input_snapshot(&ctx.chat_id.0.to_string())
        .await
    else {
        return Ok(HandlerAction::Done);
    };
    let text = build_custom_schedule_text(&input_state);
    let ret = input_state.return_to.clone();

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(build_custom_schedule_keyboard(&ret))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_ui_main(ctx: &CallbackContext) -> HandlerResult {
    if let Some((text, ret)) = ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now();
            (build_custom_schedule_text(input), input.return_to.clone())
        })
        .await
    {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(build_custom_schedule_keyboard(&ret))
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_ui_day(ctx: &CallbackContext) -> HandlerResult {
    if let Some(is_daily) = ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now();
            matches!(input.frequency, ScheduleFrequency::Daily)
        })
        .await
    {
        if is_daily {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("schedule.daily_no_day_needed"))
                .await?;
        } else {
            let text = t!("schedule.custom_day");
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(build_custom_day_keyboard())
                .await?;
        }
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_ui_hour(ctx: &CallbackContext) -> HandlerResult {
    if ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now()
        })
        .await
        .is_some()
    {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.custom_hour"))
            .parse_mode(ParseMode::Html)
            .reply_markup(build_custom_hour_keyboard())
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_ui_minute(ctx: &CallbackContext) -> HandlerResult {
    if ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now()
        })
        .await
        .is_some()
    {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.custom_minute"))
            .parse_mode(ParseMode::Html)
            .reply_markup(build_custom_minute_keyboard())
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_ui_tz(ctx: &CallbackContext) -> HandlerResult {
    if ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now()
        })
        .await
        .is_some()
    {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.custom_tz"))
            .parse_mode(ParseMode::Html)
            .reply_markup(build_custom_timezone_keyboard())
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_set(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let mut parts = data.split(':');
    let _ = parts.next();
    let field = parts.next();
    let value = parts.next();

    if let Some((text, ret)) = ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now();
            match (field, value) {
                (
                    Some("day"),
                    Some(v @ ("Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")),
                ) => {
                    input.day_of_week = Some(v.to_string());
                }
                (Some("hour"), Some(v)) => {
                    if let Ok(hour) = v.parse::<u8>()
                        && hour <= 23
                    {
                        input.hour = Some(hour);
                    }
                }
                (Some("minute"), Some(v)) => {
                    if let Ok(minute) = v.parse::<u8>()
                        && minute <= 59
                    {
                        input.minute = Some(minute);
                    }
                }
                (
                    Some("tz"),
                    Some(
                        v @ ("UTC"
                        | "Asia/Shanghai"
                        | "Asia/Tokyo"
                        | "Asia/Singapore"
                        | "Europe/London"
                        | "Europe/Berlin"
                        | "America/New_York"
                        | "America/Los_Angeles"),
                    ),
                ) => {
                    input.timezone = v.to_string();
                }
                _ => {}
            }
            (build_custom_schedule_text(input), input.return_to.clone())
        })
        .await
    {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(build_custom_schedule_keyboard(&ret))
            .await?;
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_confirm(ctx: &CallbackContext) -> HandlerResult {
    let Some((cron, task_type, timezone, return_to, hour, minute)) = ctx
        .state
        .with_schedule_input(&ctx.chat_id.0.to_string(), |input| {
            input.updated_at = Instant::now();
            (
                build_cron_from_custom_state(input),
                input.task_type.clone(),
                input.timezone.clone(),
                input.return_to.clone(),
                input.hour,
                input.minute,
            )
        })
        .await
    else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.session_not_found"))
            .await?;
        return Ok(HandlerAction::Done);
    };

    let Some(cron_expression) = cron else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.config_incomplete"))
            .show_alert(true)
            .await?;
        return Ok(HandlerAction::Done);
    };

    ctx.state
        .remove_schedule_input(&ctx.chat_id.0.to_string())
        .await;
    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
            task_type.clone(),
            &cron_expression,
            &timezone,
        );
        let result = manager.add_new_task(task).await;
        match result {
            Ok(_) => {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.task_added"))
                    .await?;
                if task_type == TaskType::SecurityUpdate {
                    let reboot_time =
                        format!("{:02}:{:02}", hour.unwrap_or(3), minute.unwrap_or(0));
                    let bot_clone = ctx.bot.clone();
                    let chat_id_clone = ctx.chat_id;
                    tokio::spawn(async move {
                        match Operations::perform_maintenance_with_reboot_time(&reboot_time).await {
                            Ok(log) => {
                                let log_tail = if log.len() > 3000 {
                                    format!("... (Truncated)\n{}", &log[log.len() - 2000..])
                                } else {
                                    log
                                };
                                let _ = bot_clone
                                    .send_message(
                                        chat_id_clone,
                                        format!(
                                            "📋 <b>{}<b>\n\n<pre>{}</pre>",
                                            t!("schedule.security_init_log_title"),
                                            log_tail
                                        ),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                            Err(e) => {
                                let _ = bot_clone
                                    .send_message(
                                        chat_id_clone,
                                        format!(
                                            "❌ <b>{}</b>\n\n{}",
                                            t!("schedule.security_init_fail"),
                                            e
                                        ),
                                    )
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }
                    });
                }
                let back_label = if return_to == "a_geo_sched_menu" {
                    t!("schedule.back_geo_sched")
                } else if return_to == "m_sys_cmd" {
                    t!("schedule.back_sys_cmd")
                } else {
                    t!("schedule.back_sched")
                };
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("schedule.task_created", "0" => cron_expression, "1" => timezone),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback(back_label, &return_to),
                    ]]))
                    .await?;
            }
            Err(e) => {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.add_fail"))
                    .show_alert(true)
                    .await?;
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!("❌ {}: {}", t!("schedule.add_fail"), e),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.scheduler_not_init"))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_custom_cancel(ctx: &CallbackContext) -> HandlerResult {
    let return_to = ctx
        .state
        .schedule_input_snapshot(&ctx.chat_id.0.to_string())
        .await
        .map(|s| s.return_to.clone())
        .unwrap_or_else(|| "s_add_menu".to_string());
    ctx.state
        .remove_schedule_input(&ctx.chat_id.0.to_string())
        .await;
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text(t!("schedule.custom_cancelled"))
        .await?;
    Ok(HandlerAction::Redirect(return_to))
}

pub(super) async fn handle_add_template(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let template = data.strip_prefix("s_add:").unwrap_or(data);
    let (task_type, cron) = match template {
        "reboot_daily_3" => (
            aegis::core::system::scheduler::task_types::TaskType::Reboot,
            "0 3 * * *",
        ),
        "reload_daily_4" => (
            aegis::core::system::scheduler::task_types::TaskType::ReloadCore,
            "0 4 * * *",
        ),
        _ => (
            aegis::core::system::scheduler::task_types::TaskType::GeoUpdate,
            "0 4 * * *",
        ),
    };

    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let task = aegis::core::system::scheduler::ScheduledTask::new(task_type, cron);
        let _ = manager.add_new_task(task).await;
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.task_added"))
            .await?;

        return Ok(HandlerAction::Redirect("m_sched".to_string()));
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_del_menu(ctx: &CallbackContext) -> HandlerResult {
    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let state = manager.state.lock().await;
        let mut buttons = Vec::new();
        for (i, task) in state.tasks.iter().enumerate() {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("{}. {}", i + 1, task.task_type.get_display_name()),
                format!("s_del:{}", i),
            )]);
        }
        buttons.push(vec![InlineKeyboardButton::callback(
            t!("menu.back"),
            "m_sched",
        )]);
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.del_menu_title"))
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_del_select(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let idx: usize = data
        .strip_prefix("s_del:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let state = manager.state.lock().await;
        if let Err(e) = utils::validate_idx(idx, state.tasks.len(), &t!("schedule.task_label")) {
            drop(state);
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(format!("❌ {}", e))
                .await?;
            return Ok(HandlerAction::Redirect(data.to_string()));
        }
        if let Some(task) = state.tasks.get(idx) {
            let task_name = task.task_type.get_display_name().to_string();
            drop(state);

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("schedule.confirm_delete"),
                    format!("s_del_confirm:{}", idx),
                )],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.custom_cancel"),
                    "s_del_menu",
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("schedule.del_confirm_title", "0" => utils::escape_html(&task_name)),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        } else {
            drop(state);
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("schedule.task_not_found"))
                .await?;
        }
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_del_confirm(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let idx: usize = data
        .strip_prefix("s_del_confirm:")
        .unwrap()
        .parse()
        .unwrap_or(0);
    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let _ = manager.remove_task_at(idx).await;
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.task_deleted"))
            .show_alert(true)
            .await?;

        return Ok(HandlerAction::Redirect("m_sched".to_string()));
    }

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_geo_sched_menu(ctx: &CallbackContext) -> HandlerResult {
    let geo_info = if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let s = manager.state.lock().await;
        let geo_tasks: Vec<_> = s
            .tasks
            .iter()
            .filter(|t| t.task_type == TaskType::GeoUpdate)
            .collect();
        if geo_tasks.is_empty() {
            t!("schedule.geo_scheduled_empty").to_string()
        } else {
            let mut info = t!("schedule.geo_scheduled_title").to_string();
            for (i, t) in geo_tasks.iter().enumerate() {
                info.push_str(&format!(
                    "{}. {}: <code>{}</code> | TZ: <code>{}</code>\n",
                    i + 1,
                    t!("schedule.cron_label"),
                    t.cron_expression,
                    t.timezone
                ));
            }
            info
        }
    } else {
        t!("schedule.schedule_summary_init_fail").to_string()
    };

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                format!("🟢 {}", t!("schedule.freq_daily")),
                "s_custom:geo:daily",
            ),
            InlineKeyboardButton::callback(
                format!("🟢 {}", t!("schedule.freq_weekly")),
                "s_custom:geo:weekly",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("schedule.geo_stop"),
            "geo_sched_off",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.back_geo"),
            "a_geo_menu",
        )],
    ]);

    ctx.bot
        .edit_message_text(
            ctx.chat_id,
            ctx.msg_id,
            t!("schedule.geo_sched_menu_title", "0" => geo_info),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_geo_sched_off(ctx: &CallbackContext) -> HandlerResult {
    if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
        let mut state_lock = manager.state.lock().await;
        let mut removed = false;
        for i in (0..state_lock.tasks.len()).rev() {
            if state_lock.tasks[i].task_type == TaskType::GeoUpdate {
                state_lock.tasks.remove(i);
                removed = true;
            }
        }
        let _ = state_lock.save_to_file(&manager.state_path);
        drop(state_lock);
        let _ = manager.start_all_tasks().await;

        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(if removed {
                t!("schedule.geo_stopped")
            } else {
                t!("schedule.geo_stop_info")
            })
            .await?;

        return Ok(HandlerAction::Redirect("a_geo_sched_menu".to_string()));
    } else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text(t!("schedule.scheduler_not_init"))
            .await?;
    }

    Ok(HandlerAction::Done)
}
