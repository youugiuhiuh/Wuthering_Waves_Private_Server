use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use crate::logic;
use crate::logic::scheduler::TaskType;
use rust_i18n::t;
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::*;

pub(crate) fn schedule_task_name(task_type: &TaskType, lang: &str) -> String {
    match task_type {
        TaskType::Unknown => t!("schedule.task_type.unknown", locale = lang).to_string(),
        TaskType::Reboot => t!("schedule.task_type.reboot", locale = lang).to_string(),
        TaskType::GeoUpdate => t!("schedule.task_type.geo_update", locale = lang).to_string(),
        TaskType::ReloadCore => t!("schedule.task_type.reload_core", locale = lang).to_string(),
    }
}

pub(crate) fn schedule_frequency_name(frequency: &ScheduleFrequency, lang: &str) -> String {
    match frequency {
        ScheduleFrequency::Daily => t!("schedule.daily", locale = lang).to_string(),
        ScheduleFrequency::Weekly => t!("schedule.weekly", locale = lang).to_string(),
    }
}

pub(crate) fn weekday_label(day: &str, lang: &str) -> String {
    match day {
        "Mon" => t!("schedule.weekday.mon", locale = lang).to_string(),
        "Tue" => t!("schedule.weekday.tue", locale = lang).to_string(),
        "Wed" => t!("schedule.weekday.wed", locale = lang).to_string(),
        "Thu" => t!("schedule.weekday.thu", locale = lang).to_string(),
        "Fri" => t!("schedule.weekday.fri", locale = lang).to_string(),
        "Sat" => t!("schedule.weekday.sat", locale = lang).to_string(),
        "Sun" => t!("schedule.weekday.sun", locale = lang).to_string(),
        _ => t!("schedule.weekday.none", locale = lang).to_string(),
    }
}

pub(crate) fn timezone_label(timezone: &str, lang: &str) -> String {
    match timezone {
        "UTC" => t!("schedule.tz_labels.UTC", locale = lang).to_string(),
        "Asia/Shanghai" => t!("schedule.tz_labels.Asia/Shanghai", locale = lang).to_string(),
        "Asia/Tokyo" => t!("schedule.tz_labels.Asia/Tokyo", locale = lang).to_string(),
        "Asia/Singapore" => t!("schedule.tz_labels.Asia/Singapore", locale = lang).to_string(),
        "Europe/London" => t!("schedule.tz_labels.Europe/London", locale = lang).to_string(),
        "Europe/Berlin" => t!("schedule.tz_labels.Europe/Berlin", locale = lang).to_string(),
        "America/New_York" => t!("schedule.tz_labels.America/New_York", locale = lang).to_string(),
        "America/Los_Angeles" => {
            t!("schedule.tz_labels.America/Los_Angeles", locale = lang).to_string()
        }
        _ => t!("schedule.tz_labels.custom", locale = lang).to_string(),
    }
}

pub(crate) fn build_custom_schedule_text(input: &ScheduleInputState, lang: &str) -> String {
    let task = schedule_task_name(&input.task_type, lang);
    let freq = schedule_frequency_name(&input.frequency, lang);
    let timezone = input.timezone.as_str();
    let timezone_text = timezone_label(timezone, lang);
    let day = input
        .day_of_week
        .as_deref()
        .map(|d| weekday_label(d, lang))
        .unwrap_or_else(|| t!("schedule.weekday.none", locale = lang).to_string());
    let hour = input
        .hour
        .map(|h| format!("{:02}", h))
        .unwrap_or_else(|| "--".to_string());
    let minute = input
        .minute
        .map(|m| format!("{:02}", m))
        .unwrap_or_else(|| "--".to_string());

    let day_line = if matches!(input.frequency, ScheduleFrequency::Weekly) {
        t!("schedule.custom_day_line", locale = lang).replace("%day%", &day)
    } else {
        String::new()
    };

    t!("schedule.custom_config", locale = lang)
        .replace("%task%", &task)
        .replace("%freq%", &freq)
        .replace("%day_line%", &day_line)
        .replace("%tz_label%", &timezone_text)
        .replace("%tz%", timezone)
        .replace("%hour%", &hour)
        .replace("%minute%", &minute)
}

pub(crate) fn build_custom_schedule_keyboard(return_to: &str, lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.custom_select_day", locale = lang),
                "s_custom_ui:day",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.custom_select_hour", locale = lang),
                "s_custom_ui:hour",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.custom_select_minute", locale = lang),
                "s_custom_ui:minute",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_select_tz", locale = lang),
            "s_custom_ui:tz",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_confirm", locale = lang),
            "s_custom_confirm",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_cancel", locale = lang),
            "s_custom_cancel",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.back", locale = lang),
            return_to,
        )],
    ])
}

pub(crate) fn build_custom_day_keyboard(lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.weekday.mon", locale = lang),
                "s_custom_set:day:Mon",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.weekday.tue", locale = lang),
                "s_custom_set:day:Tue",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.weekday.wed", locale = lang),
                "s_custom_set:day:Wed",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.weekday.thu", locale = lang),
                "s_custom_set:day:Thu",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.weekday.fri", locale = lang),
                "s_custom_set:day:Fri",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.weekday.sat", locale = lang),
                "s_custom_set:day:Sat",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.weekday.sun", locale = lang),
                "s_custom_set:day:Sun",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("schedule.back", locale = lang),
            "s_custom_ui:main",
        )],
    ])
}

pub(crate) fn build_custom_hour_keyboard() -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in (0u8..24).collect::<Vec<_>>().chunks(6) {
        let row = chunk
            .iter()
            .map(|h| {
                InlineKeyboardButton::callback(
                    format!("{:02}", h),
                    format!("s_custom_set:hour:{:02}", h),
                )
            })
            .collect::<Vec<_>>();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回",
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn build_custom_minute_keyboard() -> InlineKeyboardMarkup {
    let minute_points = [0u8, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55];
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in minute_points.chunks(4) {
        let row = chunk
            .iter()
            .map(|m| {
                InlineKeyboardButton::callback(
                    format!("{:02}", m),
                    format!("s_custom_set:minute:{:02}", m),
                )
            })
            .collect::<Vec<_>>();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回",
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn build_custom_timezone_keyboard(lang: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.utc", locale = lang),
                "s_custom_set:tz:UTC",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.china", locale = lang),
                "s_custom_set:tz:Asia/Shanghai",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.tokyo", locale = lang),
                "s_custom_set:tz:Asia/Tokyo",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.singapore", locale = lang),
                "s_custom_set:tz:Asia/Singapore",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.london", locale = lang),
                "s_custom_set:tz:Europe/London",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.berlin", locale = lang),
                "s_custom_set:tz:Europe/Berlin",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.new_york", locale = lang),
                "s_custom_set:tz:America/New_York",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.tz_buttons.los_angeles", locale = lang),
                "s_custom_set:tz:America/Los_Angeles",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("schedule.back", locale = lang),
            "s_custom_ui:main",
        )],
    ])
}

pub(crate) fn build_cron_from_custom_state(input: &ScheduleInputState) -> Option<String> {
    let hour = input.hour?;
    let minute = input.minute?;
    match input.frequency {
        ScheduleFrequency::Daily => Some(format!("{} {} * * *", minute, hour)),
        ScheduleFrequency::Weekly => input
            .day_of_week
            .as_ref()
            .map(|d| format!("{} {} * * {}", minute, hour, d)),
    }
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let lang = ctx.state.language().await;
    let data = ctx.data.as_str();

    match data {
        "m_sched" => {
            ctx.state.remove_schedule_input(ctx.chat_id).await;
            let summary = if let Some(manager) = logic::scheduler::get_manager().await {
                manager.get_summary().await
            } else {
                t!("schedule.scheduler_not_init", locale = &lang).to_string()
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("schedule.add", locale = &lang),
                        "s_add_menu",
                    ),
                    InlineKeyboardButton::callback(
                        t!("schedule.remove", locale = &lang),
                        "s_del_menu",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.back", locale = &lang),
                    "m_settings",
                )],
            ]);

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, summary)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "s_add_menu" => {
            ctx.state.remove_schedule_input(ctx.chat_id).await;
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("schedule.daily_reboot_core", locale = &lang),
                    "s_add:reload_daily_4",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.custom_time", locale = &lang),
                    "s_add_custom_menu",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.back", locale = &lang),
                    "s_add_menu",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("schedule.add_title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "s_add_custom_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("schedule.task_type.geo_update", locale = &lang),
                        "s_custom:geo:daily",
                    ),
                    InlineKeyboardButton::callback(
                        t!("schedule.weekly", locale = &lang),
                        "s_custom:geo:weekly",
                    ),
                ],
                vec![
                    InlineKeyboardButton::callback(
                        t!("schedule.task_type.reload_core", locale = &lang),
                        "s_custom:reload:daily",
                    ),
                    InlineKeyboardButton::callback(
                        t!("schedule.weekly", locale = &lang),
                        "s_custom:reload:weekly",
                    ),
                ],
                vec![
                    InlineKeyboardButton::callback(
                        t!("schedule.task_type.reboot", locale = &lang),
                        "s_custom:reboot:daily",
                    ),
                    InlineKeyboardButton::callback(
                        t!("schedule.weekly", locale = &lang),
                        "s_custom:reboot:weekly",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.back", locale = &lang),
                    "s_add_menu",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("schedule.custom_title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        d if d.starts_with("s_custom:") => {
            let mut parts = d.split(':');
            let _prefix = parts.next();
            let task_part = parts.next();
            let freq_part = parts.next();

            let (task_type, frequency) = match (task_part, freq_part) {
                (Some("geo"), Some("daily")) => (TaskType::GeoUpdate, ScheduleFrequency::Daily),
                (Some("geo"), Some("weekly")) => (TaskType::GeoUpdate, ScheduleFrequency::Weekly),
                (Some("reload"), Some("daily")) => (TaskType::ReloadCore, ScheduleFrequency::Daily),
                (Some("reload"), Some("weekly")) => {
                    (TaskType::ReloadCore, ScheduleFrequency::Weekly)
                }
                (Some("reboot"), Some("daily")) => (TaskType::Reboot, ScheduleFrequency::Daily),
                (Some("reboot"), Some("weekly")) => (TaskType::Reboot, ScheduleFrequency::Weekly),
                _ => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("schedule.custom_invalid", locale = &lang))
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
                    ctx.chat_id,
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

            let Some(input_state) = ctx.state.schedule_input_snapshot(ctx.chat_id).await else {
                return Ok(HandlerAction::Done);
            };
            let text = build_custom_schedule_text(&input_state, &lang);
            let ret = input_state.return_to.clone();

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(build_custom_schedule_keyboard(&ret, &lang))
                .await?;
        }
        "s_custom_ui:main" => {
            if let Some((text, ret)) = ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| {
                    input.updated_at = Instant::now();
                    (
                        build_custom_schedule_text(input, &lang),
                        input.return_to.clone(),
                    )
                })
                .await
            {
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_schedule_keyboard(&ret, &lang))
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        "s_custom_ui:day" => {
            if let Some(is_daily) = ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| {
                    input.updated_at = Instant::now();
                    matches!(input.frequency, ScheduleFrequency::Daily)
                })
                .await
            {
                if is_daily {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("schedule.daily_no_day", locale = &lang))
                        .await?;
                } else {
                    let text = t!("schedule.custom_select_day", locale = &lang);
                    ctx.bot
                        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(build_custom_day_keyboard(&lang))
                        .await?;
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        "s_custom_ui:hour" => {
            if ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| input.updated_at = Instant::now())
                .await
                .is_some()
            {
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("schedule.custom_select_hour", locale = &lang),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_hour_keyboard())
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        "s_custom_ui:minute" => {
            if ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| input.updated_at = Instant::now())
                .await
                .is_some()
            {
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("schedule.custom_select_minute", locale = &lang),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_minute_keyboard())
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        "s_custom_ui:tz" => {
            if ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| input.updated_at = Instant::now())
                .await
                .is_some()
            {
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("schedule.custom_select_tz", locale = &lang),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_timezone_keyboard(&lang))
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        d if d.starts_with("s_custom_set:") => {
            let mut parts = d.split(':');
            let _ = parts.next();
            let field = parts.next();
            let value = parts.next();

            if let Some((text, ret)) = ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| {
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
                    (
                        build_custom_schedule_text(input, &lang),
                        input.return_to.clone(),
                    )
                })
                .await
            {
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_schedule_keyboard(&ret, &lang))
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
            }
        }
        "s_custom_confirm" => {
            let Some((cron, task_type, timezone, return_to)) = ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| {
                    input.updated_at = Instant::now();
                    (
                        build_cron_from_custom_state(input),
                        input.task_type.clone(),
                        input.timezone.clone(),
                        input.return_to.clone(),
                    )
                })
                .await
            else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_missing", locale = &lang))
                    .await?;
                return Ok(HandlerAction::Done);
            };

            let Some(cron_expression) = cron else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.custom_incomplete", locale = &lang))
                    .show_alert(true)
                    .await?;
                return Ok(HandlerAction::Done);
            };

            ctx.state.remove_schedule_input(ctx.chat_id).await;
            if let Some(manager) = logic::scheduler::get_manager().await {
                let task = logic::scheduler::ScheduledTask::new_with_timezone(
                    task_type,
                    &cron_expression,
                    &timezone,
                );
                let result = manager
                    .add_new_task(ctx.bot.clone(), ctx.state.admin_id(), task)
                    .await;
                match result {
                    Ok(_) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("schedule.task_added", locale = &lang))
                            .await?;
                        let back_label = if return_to == "a_geo_sched_menu" {
                            t!("schedule.back_geo", locale = &lang).to_string()
                        } else {
                            t!("schedule.back_schedule", locale = &lang).to_string()
                        };
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                t!("schedule.custom_created", locale = &lang)
                                    .replace("%cron%", &cron_expression)
                                    .replace("%tz%", &timezone),
                            )
                            .parse_mode(ParseMode::Html)
                            .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                                InlineKeyboardButton::callback(&back_label, &return_to),
                            ]]))
                            .await?;
                    }
                    Err(e) => {
                        ctx.bot
                            .answer_callback_query(ctx.q.id.clone())
                            .text(t!("schedule.task_add_failed", locale = &lang))
                            .show_alert(true)
                            .await?;
                        ctx.bot
                            .edit_message_text(ctx.chat_id, ctx.msg_id, format!("❌ {}", e))
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.scheduler_not_init", locale = &lang))
                    .await?;
            }
        }
        "s_custom_cancel" => {
            let return_to = ctx
                .state
                .schedule_input_snapshot(ctx.chat_id)
                .await
                .map(|s| s.return_to.clone())
                .unwrap_or_else(|| "s_add_menu".to_string());
            ctx.state.remove_schedule_input(ctx.chat_id).await;
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("schedule.custom_cancelled", locale = &lang))
                .await?;
            return Ok(HandlerAction::Redirect(return_to));
        }
        d if d.starts_with("s_add:") => {
            let template = d.strip_prefix("s_add:").unwrap_or(d);
            let (task_type, cron) = match template {
                "reboot_daily_3" => (logic::scheduler::task_types::TaskType::Reboot, "0 3 * * *"),
                "reload_daily_4" => (
                    logic::scheduler::task_types::TaskType::ReloadCore,
                    "0 4 * * *",
                ),
                _ => (
                    logic::scheduler::task_types::TaskType::GeoUpdate,
                    "0 4 * * *",
                ),
            };

            if let Some(manager) = logic::scheduler::get_manager().await {
                let task = logic::scheduler::ScheduledTask::new(task_type, cron);
                let _ = manager
                    .add_new_task(ctx.bot.clone(), ctx.state.admin_id(), task)
                    .await;
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.task_added", locale = &lang))
                    .await?;

                return Ok(HandlerAction::Redirect("m_sched".to_string()));
            }
        }
        "s_del_menu" => {
            if let Some(manager) = logic::scheduler::get_manager().await {
                let state = manager.state.lock().await;
                let mut buttons = Vec::new();
                for (i, task) in state.tasks.iter().enumerate() {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("{}. {}", i + 1, task.task_type.get_display_name(&lang)),
                        format!("s_del:{}", i),
                    )]);
                }
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("schedule.back", locale = &lang),
                    "m_sched",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("schedule.task_remove_title", locale = &lang),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }
        }
        d if d.starts_with("s_del:") => {
            let idx: usize = d.strip_prefix("s_del:").unwrap_or("0").parse().unwrap_or(0);

            if let Some(manager) = logic::scheduler::get_manager().await {
                let state = manager.state.lock().await;
                if idx >= state.tasks.len() {
                    drop(state);
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("schedule.task_not_found", locale = &lang))
                        .await?;
                    return Ok(HandlerAction::Redirect(d.to_string()));
                }
                if let Some(task) = state.tasks.get(idx) {
                    let task_name = task.task_type.get_display_name(&lang);
                    drop(state);

                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            t!("schedule.confirm_delete", locale = &lang),
                            format!("s_del_confirm:{}", idx),
                        )],
                        vec![InlineKeyboardButton::callback(
                            t!("schedule.cancel_delete", locale = &lang),
                            "s_del_menu",
                        )],
                    ]);

                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("schedule.confirm_delete_title", locale = &lang)
                                .replace("%task%", &task_name),
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                } else {
                    drop(state);
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(t!("schedule.task_not_found", locale = &lang))
                        .await?;
                }
            }
        }
        d if d.starts_with("s_del_confirm:") => {
            let idx: usize = d
                .strip_prefix("s_del_confirm:")
                .unwrap()
                .parse()
                .unwrap_or(0);
            if let Some(manager) = logic::scheduler::get_manager().await {
                let _ = manager
                    .remove_task_at(ctx.bot.clone(), ctx.state.admin_id(), idx)
                    .await;
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.task_removed", locale = &lang))
                    .show_alert(true)
                    .await?;

                return Ok(HandlerAction::Redirect("m_sched".to_string()));
            }
        }
        "a_geo_sched_menu" => {
            let geo_info = if let Some(manager) = logic::scheduler::get_manager().await {
                let s = manager.state.lock().await;
                let geo_tasks: Vec<_> = s
                    .tasks
                    .iter()
                    .filter(|t| t.task_type == TaskType::GeoUpdate)
                    .collect();
                if geo_tasks.is_empty() {
                    t!("geo.no_tasks", locale = &lang).to_string()
                } else {
                    let mut info = t!("geo.sched_tasks", locale = &lang).to_string();
                    for (i, t) in geo_tasks.iter().enumerate() {
                        info.push_str(&format!(
                            "{}. Cron: <code>{}</code> | TZ: <code>{}</code>\n",
                            i + 1,
                            t.cron_expression,
                            t.timezone
                        ));
                    }
                    info
                }
            } else {
                t!("schedule.scheduler_not_init", locale = &lang).to_string()
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("schedule.daily", locale = &lang),
                        "s_custom:geo:daily",
                    ),
                    InlineKeyboardButton::callback(
                        t!("schedule.weekly", locale = &lang),
                        "s_custom:geo:weekly",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("geo.stop", locale = &lang),
                    "geo_sched_off",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "a_geo_menu",
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("geo.sched_title", locale = &lang).replace("%info%", &geo_info),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "geo_sched_off" => {
            if let Some(manager) = logic::scheduler::get_manager().await {
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
                let _ = manager
                    .start_all_tasks(ctx.bot.clone(), ctx.state.admin_id())
                    .await;

                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(if removed {
                        t!("geo.stopped", locale = &lang)
                    } else {
                        t!("geo.no_geo_task", locale = &lang)
                    })
                    .await?;

                return Ok(HandlerAction::Redirect("a_geo_sched_menu".to_string()));
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.scheduler_not_init", locale = &lang))
                    .await?;
            }
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }

    Ok(HandlerAction::Done)
}
