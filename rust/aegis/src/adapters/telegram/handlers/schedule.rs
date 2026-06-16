use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use crate::utils;
use aegis::core::system::operations::Operations;
use aegis::core::system::scheduler::TaskType;
use rust_i18n::t;
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::*;

pub(crate) fn schedule_task_name(task_type: &TaskType) -> String {
    match task_type {
        TaskType::Unknown => t!("schedule.task_type_unknown").to_string(),
        TaskType::Reboot => t!("schedule.task_type_reboot").to_string(),
        TaskType::GeoUpdate => t!("schedule.task_type_geo").to_string(),
        TaskType::ReloadCore => t!("schedule.task_type_reload").to_string(),
        TaskType::SecurityUpdate => t!("schedule.task_type_security").to_string(),
    }
}

pub(crate) fn schedule_frequency_name(frequency: &ScheduleFrequency) -> String {
    match frequency {
        ScheduleFrequency::Daily => t!("schedule.freq_daily").to_string(),
        ScheduleFrequency::Weekly => t!("schedule.freq_weekly").to_string(),
    }
}

pub(crate) fn weekday_label(day: &str) -> String {
    match day {
        "Mon" => t!("schedule.monday").to_string(),
        "Tue" => t!("schedule.tuesday").to_string(),
        "Wed" => t!("schedule.wednesday").to_string(),
        "Thu" => t!("schedule.thursday").to_string(),
        "Fri" => t!("schedule.friday").to_string(),
        "Sat" => t!("schedule.saturday").to_string(),
        "Sun" => t!("schedule.sunday").to_string(),
        _ => t!("schedule.not_selected").to_string(),
    }
}

pub(crate) fn timezone_label(timezone: &str) -> String {
    match timezone {
        "UTC" => t!("schedule.timezone_utc").to_string(),
        "Asia/Shanghai" => t!("schedule.timezone_cn").to_string(),
        "Asia/Tokyo" => t!("schedule.timezone_jp").to_string(),
        "Asia/Singapore" => t!("schedule.timezone_sg").to_string(),
        "Europe/London" => t!("schedule.timezone_uk").to_string(),
        "Europe/Berlin" => t!("schedule.timezone_de").to_string(),
        "America/New_York" => t!("schedule.timezone_ny").to_string(),
        "America/Los_Angeles" => t!("schedule.timezone_la").to_string(),
        _ => t!("schedule.timezone_custom").to_string(),
    }
}

pub(crate) fn build_custom_schedule_text(input: &ScheduleInputState) -> String {
    let task = schedule_task_name(&input.task_type);
    let freq = schedule_frequency_name(&input.frequency);
    let timezone = input.timezone.as_str();
    let timezone_text = timezone_label(timezone);
    let day = input
        .day_of_week
        .as_deref()
        .map(weekday_label)
        .unwrap_or_else(|| t!("schedule.not_selected").to_string());
    let hour = input
        .hour
        .map(|h| format!("{:02}", h))
        .unwrap_or_else(|| "--".to_string());
    let minute = input
        .minute
        .map(|m| format!("{:02}", m))
        .unwrap_or_else(|| "--".to_string());

    let day_line = if matches!(input.frequency, ScheduleFrequency::Weekly) {
        format!("\n{}", t!("schedule.day_fmt", "0" => day))
    } else {
        String::new()
    };

    t!("schedule.custom_text", "0" => task, "1" => freq, "2" => day_line, "3" => timezone_text, "4" => timezone, "5" => hour, "6" => minute).to_string()
}

pub(crate) fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("schedule.day_label"), "s_custom_ui:day"),
            InlineKeyboardButton::callback(t!("schedule.hour_label"), "s_custom_ui:hour"),
            InlineKeyboardButton::callback(t!("schedule.minute_label"), "s_custom_ui:minute"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("schedule.tz_label"),
            "s_custom_ui:tz",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_confirm"),
            "s_custom_confirm",
        )],
        vec![InlineKeyboardButton::callback(
            t!("schedule.custom_cancel"),
            "s_custom_cancel",
        )],
        vec![InlineKeyboardButton::callback(t!("menu.back"), return_to)],
    ])
}

pub(crate) fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("schedule.monday"), "s_custom_set:day:Mon"),
            InlineKeyboardButton::callback(t!("schedule.tuesday"), "s_custom_set:day:Tue"),
            InlineKeyboardButton::callback(t!("schedule.wednesday"), "s_custom_set:day:Wed"),
            InlineKeyboardButton::callback(t!("schedule.thursday"), "s_custom_set:day:Thu"),
        ],
        vec![
            InlineKeyboardButton::callback(t!("schedule.friday"), "s_custom_set:day:Fri"),
            InlineKeyboardButton::callback(t!("schedule.saturday"), "s_custom_set:day:Sat"),
            InlineKeyboardButton::callback(t!("schedule.sunday"), "s_custom_set:day:Sun"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
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
        t!("menu.back"),
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
        t!("menu.back"),
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("schedule.timezone_utc"), "s_custom_set:tz:UTC"),
            InlineKeyboardButton::callback(
                t!("schedule.timezone_cn"),
                "s_custom_set:tz:Asia/Shanghai",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.timezone_jp"),
                "s_custom_set:tz:Asia/Tokyo",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.timezone_sg"),
                "s_custom_set:tz:Asia/Singapore",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.timezone_uk"),
                "s_custom_set:tz:Europe/London",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.timezone_de"),
                "s_custom_set:tz:Europe/Berlin",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t!("schedule.timezone_ny"),
                "s_custom_set:tz:America/New_York",
            ),
            InlineKeyboardButton::callback(
                t!("schedule.timezone_la"),
                "s_custom_set:tz:America/Los_Angeles",
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.back"),
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
    let data = ctx.data.as_str();

    match data {
        "m_sched" => {
            ctx.state
                .remove_schedule_input(&ctx.chat_id.0.to_string())
                .await;
            let summary = if let Some(manager) = aegis::core::system::scheduler::get_manager().await
            {
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
        }
        "s_add_menu" => {
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
        }
        "s_add_custom_menu" => {
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
                (Some("secupd"), Some("daily")) => {
                    (TaskType::SecurityUpdate, ScheduleFrequency::Daily)
                }
                (Some("secupd"), Some("weekly")) => {
                    (TaskType::SecurityUpdate, ScheduleFrequency::Weekly)
                }
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
        }
        "s_custom_ui:main" => {
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
        }
        "s_custom_ui:day" => {
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
        }
        "s_custom_ui:hour" => {
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
        }
        "s_custom_ui:minute" => {
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
        }
        "s_custom_ui:tz" => {
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
        }
        d if d.starts_with("s_custom_set:") => {
            let mut parts = d.split(':');
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
        }
        "s_custom_confirm" => {
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
                                match Operations::perform_maintenance_with_reboot_time(&reboot_time)
                                    .await
                                {
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
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("schedule.scheduler_not_init"))
                    .await?;
            }
        }
        "s_custom_cancel" => {
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
            return Ok(HandlerAction::Redirect(return_to));
        }
        d if d.starts_with("s_add:") => {
            let template = d.strip_prefix("s_add:").unwrap_or(d);
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
        }
        "s_del_menu" => {
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
        }
        d if d.starts_with("s_del:") => {
            let idx: usize = d.strip_prefix("s_del:").unwrap_or("0").parse().unwrap_or(0);

            if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
                let state = manager.state.lock().await;
                if let Err(e) =
                    utils::validate_idx(idx, state.tasks.len(), &t!("schedule.task_label"))
                {
                    drop(state);
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text(format!("❌ {}", e))
                        .await?;
                    return Ok(HandlerAction::Redirect(d.to_string()));
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
        }
        d if d.starts_with("s_del_confirm:") => {
            let idx: usize = d
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
        }
        "a_geo_sched_menu" => {
            let geo_info =
                if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
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
        }
        "geo_sched_off" => {
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
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }

    Ok(HandlerAction::Done)
}
