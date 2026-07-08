use crate::adapters::common::{InlineButton, Markup, MessageContent};
use crate::core::system::scheduler::{ScheduledTask, SchedulerState, TaskType, get_manager};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::utils;
use rust_i18n::t;

struct ScheduleUI;

impl ScheduleUI {
    fn schedule_task_name(task_type: &TaskType) -> String {
        match task_type {
            TaskType::Unknown => t!("schedule.task_type_unknown").to_string(),
            TaskType::Reboot => t!("schedule.task_type_reboot").to_string(),
            TaskType::GeoUpdate => t!("schedule.task_type_geo").to_string(),
            TaskType::ReloadCore => t!("schedule.task_type_reload").to_string(),
            TaskType::SecurityUpdate => t!("schedule.task_type_security").to_string(),
        }
    }

    fn build_summary(manager: &SchedulerState) -> String {
        if manager.tasks.is_empty() {
            return t!("schedule.scheduled_empty").to_string();
        }
        let mut info = String::new();
        for (i, task) in manager.tasks.iter().enumerate() {
            let status = if task.enabled { "✅" } else { "⏸️" };
            info.push_str(&format!(
                "{}. {} <b>{}</b>\n   Cron: <code>{}</code>\n   TZ: <code>{}</code>\n\n",
                i + 1,
                status,
                Self::schedule_task_name(&task.task_type),
                task.cron_expression,
                task.timezone
            ));
        }
        info
    }
}

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    match data {
        "m_sched" => handle_sched(event).await,
        "s_add_menu" => handle_add_menu(event).await,
        d if d.starts_with("s_add:") => handle_add_template(event, d).await,
        "s_del_menu" => handle_del_menu(event).await,
        d if d.starts_with("s_del:") && !d.starts_with("s_del_confirm:") => {
            handle_del_select(event, d).await
        }
        d if d.starts_with("s_del_confirm:") => handle_del_confirm(event, d).await,
        "a_geo_sched_menu" => handle_geo_sched_menu(event).await,
        "geo_sched_off" => handle_geo_sched_off(event).await,
        _ => Ok(HandlerAction::Done),
    }
}

async fn handle_sched(event: &CallbackEvent) -> HandlerResult {
    let summary = if let Some(manager) = get_manager().await {
        let state = manager.state.lock().await;
        ScheduleUI::build_summary(&state)
    } else {
        t!("schedule.scheduler_not_init").to_string()
    };

    let markup = Markup {
        buttons: vec![
            vec![
                InlineButton {
                    text: t!("schedule.add_task").into(),
                    data: "s_add_menu".into(),
                },
                InlineButton {
                    text: t!("schedule.del_task").into(),
                    data: "s_del_menu".into(),
                },
            ],
            vec![InlineButton {
                text: t!("menu.back_settings").into(),
                data: "m_settings".into(),
            }],
        ],
    };

    let sched_text = format!("⏰ <b>{}</b>\n\n{}", t!("schedule.title"), summary);

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: sched_text,
                markup: Some(markup),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_add_menu(event: &CallbackEvent) -> HandlerResult {
    let markup = Markup {
        buttons: vec![
            vec![InlineButton {
                text: t!("schedule.daily_4am").into(),
                data: "s_add:reload_daily_4".into(),
            }],
            vec![InlineButton {
                text: t!("schedule.custom_btn").into(),
                data: "s_add_custom_menu".into(),
            }],
            vec![InlineButton {
                text: t!("menu.back").into(),
                data: "m_sched".into(),
            }],
        ],
    };

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("schedule.add_menu_title").into_owned(),
                markup: Some(markup),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_add_template(event: &CallbackEvent, data: &str) -> HandlerResult {
    let template = data.strip_prefix("s_add:").unwrap_or(data);
    let (task_type, cron) = match template {
        "reboot_daily_3" => (TaskType::Reboot, "0 3 * * *"),
        "reload_daily_4" => (TaskType::ReloadCore, "0 4 * * *"),
        _ => (TaskType::GeoUpdate, "0 4 * * *"),
    };

    if let Some(manager) = get_manager().await {
        let task = ScheduledTask::new(task_type, cron);
        match manager.add_new_task(task).await {
            Ok(msg) => {
                event
                    .adapter
                    .answer_callback(&event.target, &event.callback_id, Some(msg))
                    .await?;
                return Ok(HandlerAction::Redirect("m_sched".to_string()));
            }
            Err(e) => {
                event
                    .adapter
                    .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
                    .await?;
            }
        }
    }

    Ok(HandlerAction::Done)
}

async fn handle_del_menu(event: &CallbackEvent) -> HandlerResult {
    if let Some(manager) = get_manager().await {
        let state = manager.state.lock().await;
        let mut buttons = Vec::new();
        for (i, task) in state.tasks.iter().enumerate() {
            buttons.push(vec![InlineButton {
                text: format!(
                    "{}. {}",
                    i + 1,
                    ScheduleUI::schedule_task_name(&task.task_type)
                ),
                data: format!("s_del:{}", i),
            }]);
        }
        drop(state);
        buttons.push(vec![InlineButton {
            text: t!("menu.back").into(),
            data: "m_sched".into(),
        }]);

        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("schedule.del_menu_title").into_owned(),
                    markup: Some(Markup { buttons }),
                },
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}

async fn handle_del_select(event: &CallbackEvent, data: &str) -> HandlerResult {
    let idx: usize = data
        .strip_prefix("s_del:")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    if let Some(manager) = get_manager().await {
        let state = manager.state.lock().await;
        if let Err(e) = utils::validate_idx(idx, state.tasks.len(), &t!("schedule.task_label")) {
            drop(state);
            event
                .adapter
                .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
                .await?;
            return Ok(HandlerAction::Redirect(data.to_string()));
        }
        if let Some(task) = state.tasks.get(idx) {
            let task_name = ScheduleUI::schedule_task_name(&task.task_type);
            drop(state);

            let buttons = vec![
                vec![InlineButton {
                    text: t!("schedule.confirm_delete").into(),
                    data: format!("s_del_confirm:{}", idx),
                }],
                vec![InlineButton {
                    text: t!("schedule.custom_cancel").into(),
                    data: "s_del_menu".into(),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "schedule.del_confirm_title",
                            "0" => utils::escape_html(&task_name)
                        )
                        .into_owned(),
                        markup: Some(Markup { buttons }),
                    },
                )
                .await?;
        } else {
            event
                .adapter
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("schedule.task_not_found").into_owned()),
                )
                .await?;
        }
    }

    Ok(HandlerAction::Done)
}

async fn handle_del_confirm(event: &CallbackEvent, data: &str) -> HandlerResult {
    let idx: usize = data
        .strip_prefix("s_del_confirm:")
        .unwrap()
        .parse()
        .unwrap_or(0);
    if let Some(manager) = get_manager().await {
        match manager.remove_task_at(idx).await {
            Ok(msg) => {
                event
                    .adapter
                    .answer_callback(&event.target, &event.callback_id, Some(msg))
                    .await?;
                return Ok(HandlerAction::Redirect("m_sched".to_string()));
            }
            Err(e) => {
                event
                    .adapter
                    .answer_callback(&event.target, &event.callback_id, Some(format!("❌ {}", e)))
                    .await?;
            }
        }
    }

    Ok(HandlerAction::Done)
}

async fn handle_geo_sched_menu(event: &CallbackEvent) -> HandlerResult {
    let geo_info = if let Some(manager) = get_manager().await {
        let state = manager.state.lock().await;
        let geo_tasks: Vec<_> = state
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

    let markup = Markup {
        buttons: vec![
            vec![
                InlineButton {
                    text: format!("🟢 {}", t!("schedule.freq_daily")),
                    data: "s_custom:geo:daily".into(),
                },
                InlineButton {
                    text: format!("🟢 {}", t!("schedule.freq_weekly")),
                    data: "s_custom:geo:weekly".into(),
                },
            ],
            vec![InlineButton {
                text: t!("schedule.geo_stop").into(),
                data: "geo_sched_off".into(),
            }],
            vec![InlineButton {
                text: t!("schedule.back_geo").into(),
                data: "a_geo_menu".into(),
            }],
        ],
    };

    event
        .adapter
        .edit_message(
            &event.target,
            &event.msg_id,
            MessageContent {
                text: t!("schedule.geo_sched_menu_title", "0" => geo_info).into_owned(),
                markup: Some(markup),
            },
        )
        .await?;

    Ok(HandlerAction::Done)
}

async fn handle_geo_sched_off(event: &CallbackEvent) -> HandlerResult {
    if let Some(manager) = get_manager().await {
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

        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(if removed {
                    t!("schedule.geo_stopped").into_owned()
                } else {
                    t!("schedule.geo_stop_info").into_owned()
                }),
            )
            .await?;

        return Ok(HandlerAction::Redirect("a_geo_sched_menu".to_string()));
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("schedule.scheduler_not_init").into_owned()),
            )
            .await?;
    }

    Ok(HandlerAction::Done)
}
