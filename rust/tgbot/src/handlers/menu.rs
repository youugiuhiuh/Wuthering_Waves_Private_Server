use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::bootstrap::{BotSettings, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::logic::singbox::SingBoxInstaller;
use crate::logic::system::SystemMonitor;
use crate::logic::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use crate::utils::format_duration_human;
use rust_i18n::t;
use std::path::Path;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::core::paths::{singbox, xray};

pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let lang = "zh-CN";
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("menu.status", locale = lang), "m_mon"),
            InlineKeyboardButton::callback(t!("menu.users", locale = lang), "m_usr"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.ops_center", locale = lang),
            "m_ops_center",
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.settings", locale = lang),
            "m_settings",
        )],
    ]);
    bot.send_message(chat_id, t!("menu.main_prompt", locale = lang))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let lang = ctx.state.language().await;
    let data = ctx.data.as_str();
    match data {
        "m_main" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("menu.status", locale = &lang), "m_mon"),
                    InlineKeyboardButton::callback(t!("menu.users", locale = &lang), "m_usr"),
                ],
                vec![
                    InlineKeyboardButton::callback(
                        t!("menu.ops_center", locale = &lang),
                        "m_ops_center",
                    ),
                    InlineKeyboardButton::callback(
                        t!("menu.settings", locale = &lang),
                        "m_settings",
                    ),
                ],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.main", locale = &lang))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_ops_center" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("ops.net_opt", locale = &lang), "m_net_opt"),
                    InlineKeyboardButton::callback(
                        t!("ops.security", locale = &lang),
                        "m_security",
                    ),
                ],
                vec![
                    InlineKeyboardButton::callback(t!("ops.sys_cmd", locale = &lang), "m_sys_cmd"),
                    InlineKeyboardButton::callback(t!("ops.log", locale = &lang), "m_log"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_main", locale = &lang),
                    "m_main",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("menu.ops_center", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_settings" => {
            let timeout = ctx.state.session_timeout_secs().await;
            let timeout_label = format!(
                "🔐 {} ({})",
                t!("session.title", locale = &lang),
                format_duration_human(timeout, &lang)
            );
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("settings.xray_manage", locale = &lang),
                        "a_wwps_core_menu",
                    ),
                    InlineKeyboardButton::callback(
                        t!("settings.singbox_manage", locale = &lang),
                        "a_wwps_box_menu",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("settings.schedule", locale = &lang),
                    "m_sched",
                )],
                vec![
                    InlineKeyboardButton::callback(
                        t!("settings.geo_data", locale = &lang),
                        "a_geo_menu",
                    ),
                    InlineKeyboardButton::callback(
                        t!("settings.bot_upgrade", locale = &lang),
                        "a_upgrade",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    &timeout_label,
                    "m_session_timeout",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("settings.danger_zone", locale = &lang),
                    "m_danger",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("settings.language", locale = &lang),
                    "m_language",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("settings.default_schedule", locale = &lang),
                    "m_default_schedule",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_main", locale = &lang),
                    "m_main",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("settings.title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_net_opt" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("ops.warp", locale = &lang), "m_warp"),
                    InlineKeyboardButton::callback(t!("ops.bbr3", locale = &lang), "a_bbr3"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops", locale = &lang),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("ops.net_opt_title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_security" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("ops.firewall", locale = &lang),
                    "a_fw",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops", locale = &lang),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("ops.security_title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_sys_cmd" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("ops.reboot", locale = &lang),
                        "a_sys_reboot",
                    ),
                    InlineKeyboardButton::callback(t!("ops.reload", locale = &lang), "a_reload"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("ops.auto_update", locale = &lang),
                    "a_sys_maint",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops", locale = &lang),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("ops.sys_cmd_title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_geo_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("geo.update_now", locale = &lang),
                    "a_geo",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("geo.auto_schedule", locale = &lang),
                    "a_geo_sched_menu",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("geo.title", locale = &lang))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_mon" => {
            let report = SystemMonitor::get_status_report()
                .await
                .unwrap_or_else(|e| {
                    t!("misc.internal_error", locale = &lang).replace("%error%", &e.to_string())
                });
            let (_wwps_core, _wwps_box) = SystemMonitor::get_core_status().await;

            let status_text = report.to_string();

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("ops.refresh", locale = &lang),
                    "m_mon",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_main", locale = &lang),
                    "m_main",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, status_text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_usr" => {
            let wwps_core_config_exists = Path::new(xray::CONF_DIR).exists();
            let singbox_config_exists = Path::new(singbox::CONF_DIR).exists();
            let mut buttons = Vec::new();

            if !wwps_core_config_exists && !singbox_config_exists {
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("users.init_wwps", locale = &lang),
                    "a_inst_base",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        t!("users.no_config", locale = &lang),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else {
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("users.xray_manage", locale = &lang),
                    "m_xray_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("users.singbox_manage", locale = &lang),
                    "m_singbox_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.back_main", locale = &lang),
                    "m_main",
                )]);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("users.title", locale = &lang))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }
        }
        "m_session_timeout" => {
            let current = ctx.state.session_timeout_secs().await;
            let options: Vec<(u64, String)> = vec![
                (5 * 60, t!("session.5_min", locale = &lang).to_string()),
                (10 * 60, t!("session.10_min", locale = &lang).to_string()),
                (30 * 60, t!("session.30_min", locale = &lang).to_string()),
                (60 * 60, t!("session.1_hour", locale = &lang).to_string()),
                (4 * 3600, t!("session.4_hours", locale = &lang).to_string()),
                (
                    12 * 3600,
                    t!("session.12_hours", locale = &lang).to_string(),
                ),
                (
                    24 * 3600,
                    t!("session.24_hours", locale = &lang).to_string(),
                ),
            ];
            let mut rows = Vec::new();
            for chunk in options.chunks(3) {
                let row: Vec<InlineKeyboardButton> = chunk
                    .iter()
                    .map(|(secs, label)| {
                        let prefix = if *secs == current { "✅ " } else { "" };
                        InlineKeyboardButton::callback(
                            format!("{}{}", prefix, label),
                            format!("set_timeout:{}", secs),
                        )
                    })
                    .collect();
                rows.push(row);
            }
            rows.push(vec![InlineKeyboardButton::callback(
                t!("menu.back_settings", locale = &lang),
                "m_settings",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("session.timeout_title", locale = &lang)
                        .replace("%current%", &format_duration_human(current, &lang)),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(InlineKeyboardMarkup::new(rows))
                .await?;
        }
        d if d.starts_with("set_timeout:") => {
            let secs: u64 = d
                .strip_prefix("set_timeout:")
                .unwrap_or("0")
                .parse()
                .unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS);
            ctx.state.set_session_timeout_secs(secs).await;
            let settings = BotSettings {
                session_timeout_secs: secs,
                ..Default::default()
            };
            if let Err(e) = settings.save() {
                log::error!(
                    "{}",
                    t!("session.save_error", locale = &lang).replace("%error%", &e.to_string())
                );
            }
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(
                    t!("session.set_success", locale = &lang)
                        .replace("%duration%", &format_duration_human(secs, &lang)),
                )
                .await?;

            return Ok(HandlerAction::Redirect("m_session_timeout".to_string()));
        }
        "m_danger" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("danger.destroy_btn", locale = &lang),
                    "a_destroy_ask",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("danger.title", locale = &lang))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("xray.update_latest", locale = &lang),
                    "a_wwps_core_latest",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("xray.select_version", locale = &lang),
                    "a_wwps_core_tags",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("xray.manage", locale = &lang))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_latest" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.upgrading", locale = &lang))
                .await?;
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(None, bot_clone.clone(), chat_id_clone)
                        .await
                {
                    let _ = bot_clone
                        .send_message(
                            chat_id_clone,
                            t!("xray.upgrade_failed", locale = "zh-CN")
                                .replace("%error%", &err.to_string()),
                        )
                        .await;
                }
            });
        }
        "a_wwps_core_tags" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.upgrading", locale = &lang))
                .await?;

            let reply =
                match WwpsCoreUpgradeConfig::from_env().and_then(WwpsCoreUpgradeManager::new) {
                    Ok(manager) => match manager.fetch_recent_tags(5).await {
                        Ok(tags) if !tags.is_empty() => {
                            let mut buttons = Vec::new();
                            for tag in tags {
                                buttons.push(vec![InlineKeyboardButton::callback(
                                    format!("⬆️ {}", tag),
                                    format!("wwps_core_tag:{}", tag),
                                )]);
                            }
                            buttons.push(vec![InlineKeyboardButton::callback(
                                t!("menu.back_main", locale = &lang),
                                "a_wwps_core_menu",
                            )]);
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    t!("xray.select_version_title", locale = &lang),
                                )
                                .reply_markup(InlineKeyboardMarkup::new(buttons))
                                .await
                        }
                        Ok(_) => {
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    t!("xray.no_versions", locale = &lang),
                                )
                                .await
                        }
                        Err(err) => {
                            ctx.bot
                                .edit_message_text(
                                    ctx.chat_id,
                                    ctx.msg_id,
                                    t!("xray.version_fetch_error", locale = &lang)
                                        .replace("%error%", &err.to_string()),
                                )
                                .await
                        }
                    },
                    Err(err) => {
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                t!("xray.config_error", locale = &lang)
                                    .replace("%error%", &err.to_string()),
                            )
                            .await
                    }
                };

            if reply.is_err() {
                let _ = ctx
                    .bot
                    .send_message(ctx.chat_id, t!("xray.network_error", locale = &lang))
                    .await;
            }
        }
        d if d.starts_with("wwps_core_tag:") => {
            let tag = d.strip_prefix("wwps_core_tag:").unwrap_or("").to_string();
            if tag.is_empty() {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("xray.version_empty", locale = &lang))
                    .await?;
                return Ok(HandlerAction::Done);
            }

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("xray.upgrading_version", locale = &lang).replace("%version%", &tag))
                .await?;

            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(Some(tag), bot_clone.clone(), chat_id_clone)
                        .await
                {
                    let _ = bot_clone
                        .send_message(
                            chat_id_clone,
                            t!("xray.upgrade_failed", locale = "zh-CN")
                                .replace("%error%", &err.to_string()),
                        )
                        .await;
                }
            });
        }
        "a_wwps_box_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("singbox.restart", locale = &lang),
                    "a_wwps_box_restart",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("singbox.view_status", locale = &lang),
                    "a_wwps_box_status",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("singbox.manage", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_box_restart" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("singbox.restarting", locale = &lang))
                .await?;

            match SingBoxInstaller::restart_service().await {
                Ok(_) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("singbox.restart_success", locale = &lang),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("singbox.restart_failed", locale = &lang)
                                .replace("%error%", &err.to_string()),
                        )
                        .await?;
                }
            }
        }
        "a_wwps_box_status" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("singbox.fetching_status", locale = &lang))
                .await?;

            match SingBoxInstaller::status().await {
                Ok(status) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("singbox.status_title", locale = &lang).replace("%status%", &status),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("singbox.status_failed", locale = &lang)
                                .replace("%error%", &err.to_string()),
                        )
                        .await?;
                }
            }
        }
        "m_default_schedule" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(
                        t!("default_schedule.frequency", locale = &lang),
                        "ds_freq",
                    ),
                    InlineKeyboardButton::callback(
                        t!("default_schedule.time", locale = &lang),
                        "ds_time",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("default_schedule.timezone", locale = &lang),
                    "ds_tz",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("default_schedule.confirm", locale = &lang),
                    "ds_confirm",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("default_schedule.title", locale = &lang),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_language" => {
            let current_lang = ctx.state.language().await.clone();
            let current_label = match current_lang.as_str() {
                "zh-CN" => "🇨🇳 中文",
                "en" => "🇺🇸 English",
                "ja" => "🇯🇵 日本語",
                _ => &current_lang,
            };
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("language.btn_zh_cn", locale = &lang),
                    "set_lang:zh-CN",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("language.btn_en", locale = &lang),
                    "set_lang:en",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("language.btn_ja", locale = &lang),
                    "set_lang:ja",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings", locale = &lang),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    t!("language.title", locale = &lang).replace("%current%", current_label),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        d if d.starts_with("set_lang:") => {
            let lang_code = d.strip_prefix("set_lang:").unwrap_or("zh-CN");
            ctx.state.set_language(lang_code).await;
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("language.set_success", locale = &lang_code))
                .await?;
            return Ok(HandlerAction::Redirect("m_language".to_string()));
        }
        _ => {}
    }
    Ok(HandlerAction::Done)
}
