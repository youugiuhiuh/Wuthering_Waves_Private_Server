//! 定时任务回调处理模块

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use std::sync::Arc;

use crate::app::state::AppState;
use crate::logic::scheduler::task_types::TaskType;
use crate::logic::scheduler::ScheduledTask;
use tgbot::core::error::{Result, AppError};

/// HTML 转义
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn validate_hash_prefix(prefix: &str) -> Result<&str> {
    if prefix.is_empty() {
        return Err(AppError::InvalidParameter("hash 前缀不能为空".to_string()));
    }
    if prefix.len() > 8 {
        return Err(AppError::InvalidParameter(format!("hash 前缀过长: {} (最大 8)", prefix.len())));
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::InvalidParameter("hash 前缀包含无效字符".to_string()));
    }
    Ok(prefix)
}

pub fn validate_idx(idx: usize, max: usize, field_name: &str) -> Result<()> {
    if idx >= max {
        return Err(AppError::InvalidParameter(format!(
            "{} 索引 {} 超出范围 (最大 {})",
            field_name,
            idx,
            max.saturating_sub(1)
        )));
    }
    Ok(())
}

/// 定时任务主菜单
pub async fn handle_sched_menu(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    state.remove_schedule_input(chat_id).await;
    let summary = if let Some(manager) = crate::logic::scheduler::get_manager().await {
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

    bot.edit_message_text(chat_id, msg_id, summary)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

/// 添加任务菜单
pub async fn handle_s_add_menu(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    state.remove_schedule_input(chat_id).await;
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "每周日 4点 维护+重启",
            "s_add:maint_sun_4",
        )],
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
    bot.edit_message_text(
        chat_id,
        msg_id,
        "➕ <b>添加快速任务</b>\n请选择预设模板:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 自定义任务菜单
pub async fn handle_s_add_custom_menu(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                "维护+重启 - 每天",
                "s_custom:maint:daily",
            ),
            InlineKeyboardButton::callback(
                "维护+重启 - 每周",
                "s_custom:maint:weekly",
            ),
        ],
        vec![
            InlineKeyboardButton::callback("Geo更新 - 每天", "s_custom:geo:daily"),
            InlineKeyboardButton::callback("Geo更新 - 每周", "s_custom:geo:weekly"),
        ],
        vec![
            InlineKeyboardButton::callback(
                "重载核心 - 每天",
                "s_custom:reload:daily",
            ),
            InlineKeyboardButton::callback(
                "重载核心 - 每周",
                "s_custom:reload:weekly",
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                "系统重启 - 每天",
                "s_custom:reboot:daily",
            ),
            InlineKeyboardButton::callback(
                "系统重启 - 每周",
                "s_custom:reboot:weekly",
            ),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "s_add_menu")],
    ]);
    bot.edit_message_text(
        chat_id,
        msg_id,
        "🧩 <b>自定义定时任务</b>\n先选择任务类型和周期:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

/// 自定义任务类型+周期选择
pub async fn handle_s_custom(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let mut parts = data.split(':');
    let _prefix = parts.next();
    let task_part = parts.next();
    let freq_part = parts.next();

    let (task_type, frequency) = match (task_part, freq_part) {
        (Some("maint"), Some("daily")) => {
            (TaskType::SystemMaintenance, crate::app::state::ScheduleFrequency::Daily)
        }
        (Some("maint"), Some("weekly")) => {
            (TaskType::SystemMaintenance, crate::app::state::ScheduleFrequency::Weekly)
        }
        (Some("geo"), Some("daily")) => {
            (TaskType::GeoUpdate, crate::app::state::ScheduleFrequency::Daily)
        }
        (Some("geo"), Some("weekly")) => {
            (TaskType::GeoUpdate, crate::app::state::ScheduleFrequency::Weekly)
        }
        (Some("reload"), Some("daily")) => {
            (TaskType::ReloadCore, crate::app::state::ScheduleFrequency::Daily)
        }
        (Some("reload"), Some("weekly")) => {
            (TaskType::ReloadCore, crate::app::state::ScheduleFrequency::Weekly)
        }
        (Some("reboot"), Some("daily")) => {
            (TaskType::Reboot, crate::app::state::ScheduleFrequency::Daily)
        }
        (Some("reboot"), Some("weekly")) => {
            (TaskType::Reboot, crate::app::state::ScheduleFrequency::Weekly)
        }
        _ => {
            bot.answer_callback_query(q.id)
                .text("❌ 无效的自定义任务模板")
                .await?;
            return Ok(());
        }
    };

    let return_to = match &task_type {
        TaskType::GeoUpdate => "a_geo_sched_menu",
        _ => "s_add_custom_menu",
    };
    state
        .insert_schedule_input(
            chat_id,
            crate::app::state::ScheduleInputState {
                updated_at: std::time::Instant::now(),
                task_type: task_type.clone(),
                frequency: frequency.clone(),
                timezone: "UTC".to_string(),
                day_of_week: None,
                hour: None,
                minute: None,
                return_to: return_to.to_string(),
            },
        )
        .await;

    let Some(input_state) = state.schedule_input_snapshot(chat_id).await else {
        return Ok(());
    };
    let text = crate::handlers::system::build_custom_schedule_text(&input_state);
    let ret = input_state.return_to.clone();

    bot.edit_message_text(chat_id, msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(crate::handlers::system::build_custom_schedule_keyboard(&ret))
        .await?;
    Ok(())
}

/// 自定义任务 UI 刷新 (日期/小时/分钟/时区选择)
pub async fn handle_s_custom_ui(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        "s_custom_ui:main" => {
            if let Some((text, ret)) = state
                .with_schedule_input(chat_id, |input| {
                    input.updated_at = std::time::Instant::now();
                    (crate::handlers::system::build_custom_schedule_text(input), input.return_to.clone())
                })
                .await
            {
                bot.edit_message_text(chat_id, msg_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(crate::handlers::system::build_custom_schedule_keyboard(&ret))
                    .await?;
            } else {
                bot.answer_callback_query(q.id)
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        "s_custom_ui:day" => {
            if let Some(is_daily) = state
                .with_schedule_input(chat_id, |input| {
                    input.updated_at = std::time::Instant::now();
                    matches!(input.frequency, crate::app::state::ScheduleFrequency::Daily)
                })
                .await
            {
                if is_daily {
                    bot.answer_callback_query(q.id)
                        .text("ℹ️ 每天任务无需选择星期")
                        .await?;
                } else {
                    let text = "📅 <b>选择每周执行的星期</b>";
                    bot.edit_message_text(chat_id, msg_id, text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(crate::handlers::system::build_custom_day_keyboard())
                        .await?;
                }
            } else {
                bot.answer_callback_query(q.id)
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        "s_custom_ui:hour" => {
            if state
                .with_schedule_input(chat_id, |input| input.updated_at = std::time::Instant::now())
                .await
                .is_some()
            {
                bot.edit_message_text(chat_id, msg_id, "🕐 <b>选择执行小时</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(crate::handlers::system::build_custom_hour_keyboard())
                    .await?;
            } else {
                bot.answer_callback_query(q.id)
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        "s_custom_ui:minute" => {
            if state
                .with_schedule_input(chat_id, |input| input.updated_at = std::time::Instant::now())
                .await
                .is_some()
            {
                bot.edit_message_text(chat_id, msg_id, "🕑 <b>选择执行分钟</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(crate::handlers::system::build_custom_minute_keyboard())
                    .await?;
            } else {
                bot.answer_callback_query(q.id)
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        "s_custom_ui:tz" => {
            if state
                .with_schedule_input(chat_id, |input| input.updated_at = std::time::Instant::now())
                .await
                .is_some()
            {
                bot.edit_message_text(chat_id, msg_id, "🌍 <b>选择任务时区</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(crate::handlers::system::build_custom_timezone_keyboard())
                    .await?;
            } else {
                bot.answer_callback_query(q.id)
                    .text("⚠️ 自定义定时会话不存在，请重新进入。")
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// 设置自定义任务参数 (日期/小时/分钟/时区)
pub async fn handle_s_custom_set(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let mut parts = data.split(':');
    let _ = parts.next();
    let field = parts.next();
    let value = parts.next();

    if let Some((text, ret)) = state
        .with_schedule_input(chat_id, |input| {
            input.updated_at = std::time::Instant::now();
            match (field, value) {
                (
                    Some("day"),
                    Some(
                        v @ ("Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun"),
                    ),
                ) => {
                    input.day_of_week = Some(v.to_string());
                }
                (Some("hour"), Some(v)) => {
                    if let Ok(hour) = v.parse::<u8>() && hour <= 23 {
                        input.hour = Some(hour);
                    }
                }
                (Some("minute"), Some(v)) => {
                    if let Ok(minute) = v.parse::<u8>() && minute <= 59 {
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
            (crate::handlers::system::build_custom_schedule_text(input), input.return_to.clone())
        })
        .await
    {
        bot.edit_message_text(chat_id, msg_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(crate::handlers::system::build_custom_schedule_keyboard(&ret))
            .await?;
    } else {
        bot.answer_callback_query(q.id)
            .text("⚠️ 自定义定时会话不存在，请重新进入。")
            .await?;
    }
    Ok(())
}

/// 确认创建自定义任务
pub async fn handle_s_custom_confirm(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let Some((cron, task_type, timezone, return_to)) = state
        .with_schedule_input(chat_id, |input| {
            input.updated_at = std::time::Instant::now();
            (
                crate::handlers::system::build_cron_from_custom_state(input),
                input.task_type.clone(),
                input.timezone.clone(),
                input.return_to.clone(),
            )
        })
        .await
    else {
        bot.answer_callback_query(q.id)
            .text("⚠️ 自定义定时会话不存在，请重新进入。")
            .await?;
        return Ok(());
    };

    let Some(cron_expression) = cron else {
        bot.answer_callback_query(q.id)
            .text("⚠️ 配置不完整，请先选择必要时间项。")
            .show_alert(true)
            .await?;
        return Ok(());
    };

    state.remove_schedule_input(chat_id).await;
    if let Some(manager) = crate::logic::scheduler::get_manager().await {
        let task = ScheduledTask::new_with_timezone(
            task_type,
            &cron_expression,
            &timezone,
        );
        let result = manager
            .add_new_task(bot.clone(), state.admin_id(), task)
            .await;
        match result {
            Ok(_) => {
                bot.answer_callback_query(q.id)
                    .text("✅ 任务添加成功")
                    .await?;
                let back_label = if return_to == "a_geo_sched_menu" {
                    "⬅️ 返回 Geo 调度"
                } else {
                    "⬅️ 返回定时任务"
                };
                bot.edit_message_text(
                    chat_id,
                    msg_id,
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
                bot.answer_callback_query(q.id)
                    .text("❌ 添加任务失败")
                    .show_alert(true)
                    .await?;
                bot.edit_message_text(
                    chat_id,
                    msg_id,
                    format!("❌ 添加任务失败: {}", e),
                )
                .await?;
            }
        }
    } else {
        bot.answer_callback_query(q.id)
            .text("❌ 调度器未初始化")
            .await?;
    }
    Ok(())
}

/// 取消自定义任务
pub async fn handle_s_custom_cancel(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let return_to = state
        .schedule_input_snapshot(chat_id)
        .await
        .map(|s| s.return_to.clone())
        .unwrap_or_else(|| "s_add_menu".to_string());
    state.remove_schedule_input(chat_id).await;
    bot.answer_callback_query(q.id.clone())
        .text("✅ 已取消自定义定时任务")
        .await?;
    let new_q = q.clone();
    let data = return_to;
    Ok(())
}

/// 添加预设任务
pub async fn handle_s_add(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let template = data.strip_prefix("s_add:").unwrap_or(data);
    let (task_type, cron) = match template {
        "maint_sun_4" => (
            crate::logic::scheduler::task_types::TaskType::SystemMaintenance,
            "0 4 * * Sun",
        ),
        "reboot_daily_3" => {
            (crate::logic::scheduler::task_types::TaskType::Reboot, "0 3 * * *")
        }
        "reload_daily_4" => (
            crate::logic::scheduler::task_types::TaskType::ReloadCore,
            "0 4 * * *",
        ),
        _ => (
            crate::logic::scheduler::task_types::TaskType::SystemMaintenance,
            "0 4 * * Sun",
        ),
    };

    if let Some(manager) = crate::logic::scheduler::get_manager().await {
        let task = ScheduledTask::new(task_type, cron);
        let _ = manager
            .add_new_task(bot.clone(), state.admin_id(), task)
            .await;
        bot.answer_callback_query(q.id.clone())
            .text("✅ 任务添加成功")
            .await?;

        let new_q = q.clone();
        q = CallbackQuery {
            data: Some("m_sched".to_string()),
            ..new_q
        };
    }
    Ok(())
}

/// 删除任务菜单
pub async fn handle_s_del_menu(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    if let Some(manager) = crate::logic::scheduler::get_manager().await {
        let state = manager.state.lock().await;
        let mut buttons = Vec::new();
        for (i, task) in state.tasks.iter().enumerate() {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("{}. {}", i + 1, task.task_type.get_display_name()),
                format!("s_del:{}", i),
            )]);
        }
        buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_sched")]);
        bot.edit_message_text(chat_id, msg_id, "➖ <b>删除任务</b>\n点击移除:")
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(buttons))
            .await?;
    }
    Ok(())
}

/// 删除任务确认
pub async fn handle_s_del(
    bot: Bot,
    q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let idx: usize = data.strip_prefix("s_del:").unwrap_or("0").parse().unwrap_or(0);

    if let Some(manager) = crate::logic::scheduler::get_manager().await {
        let state = manager.state.lock().await;
        if let Err(e) = validate_idx(idx, state.tasks.len(), "任务") {
            drop(state);
            bot.answer_callback_query(q.id.clone())
                .text(&format!("❌ {}", e))
                .await?;
            return Ok(());
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

            bot.edit_message_text(
                chat_id,
                msg_id,
                format!(
                    "⚠️ <b>删除确认</b>\n\n您确定要删除定时任务 <code>{}</code> 吗？",
                    escape_html(&task_name)
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
        } else {
            drop(state);
            bot.answer_callback_query(q.id)
                .text("❌ 任务不存在")
                .await?;
        }
    }
    Ok(())
}

/// 执行删除任务
pub async fn handle_s_del_confirm(
    bot: Bot,
    mut q: CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    let idx: usize = data
        .strip_prefix("s_del_confirm:")
        .unwrap()
        .parse()
        .unwrap_or(0);
    if let Some(manager) = crate::logic::scheduler::get_manager().await {
        let _ = manager
            .remove_task_at(bot.clone(), state.admin_id(), idx)
            .await;
        bot.answer_callback_query(q.id.clone())
            .text("✅ 任务删除成功")
            .show_alert(true)
            .await?;

        let new_q = q.clone();
        q = CallbackQuery {
            data: Some("m_sched".to_string()),
            ..new_q
        };
    }
    Ok(())
}

/// 定时任务回调分派
pub async fn dispatch_callback(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    msg_id: MessageId,
    data: &str,
    state: &Arc<AppState>,
) -> ResponseResult<()> {
    match data {
        "m_sched" => handle_sched_menu(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?,
        "s_add_menu" => handle_s_add_menu(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?,
        "s_add_custom_menu" => handle_s_add_custom_menu(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?,
        d if d.starts_with("s_custom:") => {
            handle_s_custom(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        d if d.starts_with("s_custom_ui:") => {
            handle_s_custom_ui(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        d if d.starts_with("s_custom_set:") => {
            handle_s_custom_set(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        "s_custom_confirm" => handle_s_custom_confirm(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?,
        "s_custom_cancel" => handle_s_custom_cancel(bot.clone(), q.clone(), chat_id, state.clone()).await?,
        d if d.starts_with("s_add:") => {
            handle_s_add(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        "s_del_menu" => handle_s_del_menu(bot.clone(), q.clone(), chat_id, msg_id, state.clone()).await?,
        d if d.starts_with("s_del:") => {
            handle_s_del(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        d if d.starts_with("s_del_confirm:") => {
            handle_s_del_confirm(bot.clone(), q.clone(), chat_id, msg_id, d, state.clone()).await?
        }
        _ => {}
    }
    Ok(())
}