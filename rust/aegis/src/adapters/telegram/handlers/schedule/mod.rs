use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::app::state::ScheduleFrequency;
use aegis::core::system::scheduler::TaskType;
use rust_i18n::t;

use teloxide::prelude::*;

mod handle;
mod keyboard;

pub(super) use keyboard::{build_custom_schedule_keyboard, build_custom_schedule_text};

/// Entry point for the schedule management flow.
///
/// Routes callback data to sub-handlers for creating, viewing,
/// and managing scheduled tasks.
pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "m_sched" => handle::handle_sched(ctx).await,
        "s_add_menu" => handle::handle_add_menu(ctx).await,
        "s_add_custom_menu" => handle::handle_add_custom_menu(ctx).await,
        d if d.starts_with("s_custom:") => handle::handle_custom_new(ctx, d).await,
        "s_custom_ui:main" => handle::handle_custom_ui_main(ctx).await,
        "s_custom_ui:day" => handle::handle_custom_ui_day(ctx).await,
        "s_custom_ui:hour" => handle::handle_custom_ui_hour(ctx).await,
        "s_custom_ui:minute" => handle::handle_custom_ui_minute(ctx).await,
        "s_custom_ui:tz" => handle::handle_custom_ui_tz(ctx).await,
        d if d.starts_with("s_custom_set:") => handle::handle_custom_set(ctx, d).await,
        "s_custom_confirm" => handle::handle_custom_confirm(ctx).await,
        "s_custom_cancel" => handle::handle_custom_cancel(ctx).await,
        d if d.starts_with("s_add:") => handle::handle_add_template(ctx, d).await,
        "s_del_menu" => handle::handle_del_menu(ctx).await,
        d if d.starts_with("s_del:") => handle::handle_del_select(ctx, d).await,
        d if d.starts_with("s_del_confirm:") => handle::handle_del_confirm(ctx, d).await,
        "a_geo_sched_menu" => handle::handle_geo_sched_menu(ctx).await,
        "geo_sched_off" => handle::handle_geo_sched_off(ctx).await,
        _ => {
            ctx.bot.answer_callback_query(ctx.q.id.clone()).await?;
            Ok(HandlerAction::Done)
        }
    }
}

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
