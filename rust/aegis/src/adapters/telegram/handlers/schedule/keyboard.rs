use super::{schedule_frequency_name, schedule_task_name, timezone_label, weekday_label};
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use rust_i18n::t;
use teloxide::types::InlineKeyboardButton;
use teloxide::types::InlineKeyboardMarkup;

pub(super) fn build_custom_schedule_text(input: &ScheduleInputState) -> String {
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

pub(super) fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup {
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

pub(super) fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn build_custom_hour_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn build_custom_minute_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn build_cron_from_custom_state(input: &ScheduleInputState) -> Option<String> {
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
