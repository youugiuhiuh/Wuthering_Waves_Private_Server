use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use tgbot::logic::scheduler::task_types::TaskType;

pub fn schedule_task_name(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::SystemMaintenance => "系统维护+重启",
        TaskType::Reboot => "系统重启",
        TaskType::GeoUpdate => "GeoData 更新",
        TaskType::ReloadCore => "重载核心",
    }
}

pub fn schedule_frequency_name(frequency: &ScheduleFrequency) -> &'static str {
    match frequency {
        ScheduleFrequency::Daily => "每天",
        ScheduleFrequency::Weekly => "每周",
    }
}

pub fn weekday_label(day: &str) -> &'static str {
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

pub fn timezone_label(timezone: &str) -> &'static str {
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

pub fn build_custom_schedule_text(input: &ScheduleInputState) -> String {
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

pub fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup {
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

pub fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
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

pub fn build_custom_hour_keyboard() -> InlineKeyboardMarkup {
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

pub fn build_custom_minute_keyboard() -> InlineKeyboardMarkup {
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

pub fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup {
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

pub fn build_cron_from_custom_state(input: &ScheduleInputState) -> Option<String> {
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