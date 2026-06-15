use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use crate::utils;
use aegis::core::system::operations::Operations;
use aegis::core::system::scheduler::TaskType;
use std::time::Instant;
use teloxide::prelude::*;
use teloxide::types::*;

pub(crate) fn schedule_task_name(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::Unknown => "未知任务",
        TaskType::Reboot => "系统重启",
        TaskType::GeoUpdate => "GeoData 更新",
        TaskType::ReloadCore => "重载核心",
        TaskType::SecurityUpdate => "安全更新",
    }
}

pub(crate) fn schedule_frequency_name(frequency: &ScheduleFrequency) -> &'static str {
    match frequency {
        ScheduleFrequency::Daily => "每天",
        ScheduleFrequency::Weekly => "每周",
    }
}

pub(crate) fn weekday_label(day: &str) -> &'static str {
    match day {
        "Mon" => "周一",
        "Tue" => "周二",
        "Wed" => "周三",
        "Thu" => "周四",
        "Fri" => "周五",
        "Sat" => "周六",
        "Sun" => "周日",
        _ => "未选择",
    }
}

pub(crate) fn timezone_label(timezone: &str) -> &'static str {
    match timezone {
        "UTC" => "UTC",
        "Asia/Shanghai" => "中国标准时间 (UTC+8)",
        "Asia/Tokyo" => "日本标准时间 (UTC+9)",
        "Asia/Singapore" => "新加坡时间 (UTC+8)",
        "Europe/London" => "英国时间",
        "Europe/Berlin" => "中欧时间",
        "America/New_York" => "美国东部时间",
        "America/Los_Angeles" => "美国太平洋时间",
        _ => "自定义时区",
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
        .unwrap_or("未选择");
    let hour = input
        .hour
        .map(|h| format!("{:02}", h))
        .unwrap_or_else(|| "--".to_string());
    let minute = input
        .minute
        .map(|m| format!("{:02}", m))
        .unwrap_or_else(|| "--".to_string());

    let day_line = if matches!(input.frequency, ScheduleFrequency::Weekly) {
        format!("\n📅 星期: <b>{}</b>", day)
    } else {
        String::new()
    };

    format!(
        "🧩 <b>自定义定时任务</b>\n\n📌 任务: <b>{}</b>\n🔁 周期: <b>{}</b>{}\n🌍 时区: <b>{}</b>\n   <code>{}</code>\n🕒 时间: <b>{}:{}</b>\n\n请继续点击按钮完成设置。",
        task, freq, day_line, timezone_text, timezone, hour, minute
    )
}

pub(crate) fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📅 选择星期", "s_custom_ui:day"),
            InlineKeyboardButton::callback("🕐 选择小时", "s_custom_ui:hour"),
            InlineKeyboardButton::callback("🕑 选择分钟", "s_custom_ui:minute"),
        ],
        vec![InlineKeyboardButton::callback(
            "🌍 选择时区",
            "s_custom_ui:tz",
        )],
        vec![InlineKeyboardButton::callback(
            "✅ 确认创建任务",
            "s_custom_confirm",
        )],
        vec![InlineKeyboardButton::callback("❌ 取消", "s_custom_cancel")],
        vec![InlineKeyboardButton::callback("⬅️ 返回", return_to)],
    ])
}

pub(crate) fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("周一", "s_custom_set:day:Mon"),
            InlineKeyboardButton::callback("周二", "s_custom_set:day:Tue"),
            InlineKeyboardButton::callback("周三", "s_custom_set:day:Wed"),
            InlineKeyboardButton::callback("周四", "s_custom_set:day:Thu"),
        ],
        vec![
            InlineKeyboardButton::callback("周五", "s_custom_set:day:Fri"),
            InlineKeyboardButton::callback("周六", "s_custom_set:day:Sat"),
            InlineKeyboardButton::callback("周日", "s_custom_set:day:Sun"),
        ],
        vec![InlineKeyboardButton::callback(
            "⬅️ 返回配置",
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
        "⬅️ 返回配置",
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
        "⬅️ 返回配置",
        "s_custom_ui:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("UTC", "s_custom_set:tz:UTC"),
            InlineKeyboardButton::callback("中国 (UTC+8)", "s_custom_set:tz:Asia/Shanghai"),
        ],
        vec![
            InlineKeyboardButton::callback("东京 (UTC+9)", "s_custom_set:tz:Asia/Tokyo"),
            InlineKeyboardButton::callback("新加坡 (UTC+8)", "s_custom_set:tz:Asia/Singapore"),
        ],
        vec![
            InlineKeyboardButton::callback("伦敦", "s_custom_set:tz:Europe/London"),
            InlineKeyboardButton::callback("柏林", "s_custom_set:tz:Europe/Berlin"),
        ],
        vec![
            InlineKeyboardButton::callback("纽约", "s_custom_set:tz:America/New_York"),
            InlineKeyboardButton::callback("洛杉矶", "s_custom_set:tz:America/Los_Angeles"),
        ],
        vec![InlineKeyboardButton::callback(
            "⬅️ 返回配置",
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
            ctx.state.remove_schedule_input(ctx.chat_id).await;
            let summary = if let Some(manager) = aegis::core::system::scheduler::get_manager().await
            {
                manager.get_summary().await
            } else {
                "❌ 调度器未初始化".to_string()
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("➕ 添加任务", "s_add_menu"),
                    InlineKeyboardButton::callback("➖ 删除任务", "s_del_menu"),
                ],
                vec![InlineKeyboardButton::callback("⬅️ 返回设置", "m_settings")],
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
                    "每天 4点 重启核心",
                    "s_add:reload_daily_4",
                )],
                vec![InlineKeyboardButton::callback(
                    "🕒 自定义每天/每周时间",
                    "s_add_custom_menu",
                )],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "➕ <b>添加快速任务</b>\n请选择预设模板:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "s_add_custom_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("Geo更新 - 每天", "s_custom:geo:daily"),
                    InlineKeyboardButton::callback("Geo更新 - 每周", "s_custom:geo:weekly"),
                ],
                vec![
                    InlineKeyboardButton::callback("重载核心 - 每天", "s_custom:reload:daily"),
                    InlineKeyboardButton::callback("重载核心 - 每周", "s_custom:reload:weekly"),
                ],
                vec![
                    InlineKeyboardButton::callback("系统重启 - 每天", "s_custom:reboot:daily"),
                    InlineKeyboardButton::callback("系统重启 - 每周", "s_custom:reboot:weekly"),
                ],
                vec![
                    InlineKeyboardButton::callback("安全更新 - 每天", "s_custom:secupd:daily"),
                    InlineKeyboardButton::callback("安全更新 - 每周", "s_custom:secupd:weekly"),
                ],
                vec![InlineKeyboardButton::callback("⬅️ 返回", "s_add_menu")],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    "🧩 <b>自定义定时任务</b>\n先选择任务类型和周期:",
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
                (Some("secupd"), Some("daily")) => {
                    (TaskType::SecurityUpdate, ScheduleFrequency::Daily)
                }
                (Some("secupd"), Some("weekly")) => {
                    (TaskType::SecurityUpdate, ScheduleFrequency::Weekly)
                }
                _ => {
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text("❌ 无效的自定义任务模板")
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
                .with_schedule_input(ctx.chat_id, |input| {
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
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
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
                        .text("ℹ️ 每天任务无需选择星期")
                        .await?;
                } else {
                    let text = "📅 <b>选择每周执行的星期</b>";
                    ctx.bot
                        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(build_custom_day_keyboard())
                        .await?;
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
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
                    .edit_message_text(ctx.chat_id, ctx.msg_id, "🕐 <b>选择执行小时</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_hour_keyboard())
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
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
                    .edit_message_text(ctx.chat_id, ctx.msg_id, "🕑 <b>选择执行分钟</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_minute_keyboard())
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
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
                    .edit_message_text(ctx.chat_id, ctx.msg_id, "🌍 <b>选择任务时区</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(build_custom_timezone_keyboard())
                    .await?;
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
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
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        "s_custom_confirm" => {
            let Some((cron, task_type, timezone, return_to, hour, minute)) = ctx
                .state
                .with_schedule_input(ctx.chat_id, |input| {
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
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
                return Ok(HandlerAction::Done);
            };

            let Some(cron_expression) = cron else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("⚠️ 配置不完整，请先选择必要时间项。")
                    .show_alert(true)
                    .await?;
                return Ok(HandlerAction::Done);
            };

            ctx.state.remove_schedule_input(ctx.chat_id).await;
            if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
                let task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
                    task_type.clone(),
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
                            .text("✅ 任务添加成功")
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
                                                    "📋 <b>安全更新初始配置日志</b>\n\n<pre>{}</pre>",
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
                                                format!("❌ <b>安全更新初始配置失败</b>\n\n{}", e),
                                            )
                                            .parse_mode(ParseMode::Html)
                                            .await;
                                    }
                                }
                            });
                        }
                        let back_label = if return_to == "a_geo_sched_menu" {
                            "⬅️ 返回 Geo 调度"
                        } else if return_to == "m_sys_cmd" {
                            "⬅️ 返回系统指令"
                        } else {
                            "⬅️ 返回定时任务"
                        };
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                format!(
                                    "✅ 任务已创建\nCron: <code>{}</code>\nTZ: <code>{}</code>",
                                    cron_expression, timezone
                                ),
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
                            .text("❌ 添加任务失败")
                            .show_alert(true)
                            .await?;
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                format!("❌ 添加任务失败: {}", e),
                            )
                            .await?;
                    }
                }
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("❌ 调度器未初始化")
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
                .text("✅ 已取消自定义定时任务")
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
                let _ = manager
                    .add_new_task(ctx.bot.clone(), ctx.state.admin_id(), task)
                    .await;
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("✅ 任务添加成功")
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
                buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, "➖ <b>删除任务</b>\n点击移除:")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }
        }
        d if d.starts_with("s_del:") => {
            let idx: usize = d.strip_prefix("s_del:").unwrap_or("0").parse().unwrap_or(0);

            if let Some(manager) = aegis::core::system::scheduler::get_manager().await {
                let state = manager.state.lock().await;
                if let Err(e) = utils::validate_idx(idx, state.tasks.len(), "任务") {
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
                            "⚠️ 确认删除",
                            format!("s_del_confirm:{}", idx),
                        )],
                        vec![InlineKeyboardButton::callback("🔙 取消", "s_del_menu")],
                    ]);

                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            format!(
                                "⚠️ <b>删除确认</b>\n\n您确定要删除定时任务 <code>{}</code> 吗？",
                                utils::escape_html(&task_name)
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .reply_markup(keyboard)
                        .await?;
                } else {
                    drop(state);
                    ctx.bot
                        .answer_callback_query(ctx.q.id.clone())
                        .text("❌ 任务不存在")
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
                let _ = manager
                    .remove_task_at(ctx.bot.clone(), ctx.state.admin_id(), idx)
                    .await;
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("✅ 任务删除成功")
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
                        "📝 当前无 Geo 自动更新任务".to_string()
                    } else {
                        let mut info = "⏰ <b>当前 Geo 定时任务</b>:\n\n".to_string();
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
                    "❌ 调度器未初始化".to_string()
                };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🟢 每天", "s_custom:geo:daily"),
                    InlineKeyboardButton::callback("🟢 每周", "s_custom:geo:weekly"),
                ],
                vec![InlineKeyboardButton::callback(
                    "⛔️ 停止 Geo 自动更新",
                    "geo_sched_off",
                )],
                vec![InlineKeyboardButton::callback(
                    "⬅️ 返回 Geo 数据",
                    "a_geo_menu",
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "🌍 <b>Geo 自动更新调度</b>\n\n{}\n\n选择周期来自定义调度时间:",
                        geo_info
                    ),
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
                let _ = manager
                    .start_all_tasks(ctx.bot.clone(), ctx.state.admin_id())
                    .await;

                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(if removed {
                        "✅ 已停止 Geo 自动更新"
                    } else {
                        "ℹ️ 未找到 Geo 自动更新任务"
                    })
                    .await?;

                return Ok(HandlerAction::Redirect("a_geo_sched_menu".to_string()));
            } else {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text("❌ 调度器未初始化")
                    .await?;
            }
        }
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
        }
    }

    Ok(HandlerAction::Done)
}
