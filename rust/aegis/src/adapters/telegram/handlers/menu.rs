use super::context::{CallbackContext, HandlerAction, HandlerResult};
use crate::bootstrap::{BOT_VERSION, BotSettings, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::utils::format_duration_human;
use aegis::adapters::common::TargetId;
use aegis::core::paths::{singbox, xray};
use aegis::core::singbox::SingBoxInstaller;
use aegis::core::system::SystemMonitor;
use aegis::core::system::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use rust_i18n::t;
use std::path::Path;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(t!("menu.monitor"), "m_mon"),
            InlineKeyboardButton::callback(t!("menu.users"), "m_usr"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.ops"),
            "m_ops_center",
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.settings"),
            "m_settings",
        )],
        vec![
            InlineKeyboardButton::callback(t!("lang.zh"), "lang:zh"),
            InlineKeyboardButton::callback(t!("lang.en"), "lang:en"),
            InlineKeyboardButton::callback(t!("lang.ja"), "lang:ja"),
        ],
    ]);
    bot.send_message(
        chat_id,
        format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let data = ctx.data.as_str();
    match data {
        "m_main" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("menu.monitor"), "m_mon"),
                    InlineKeyboardButton::callback(t!("menu.users"), "m_usr"),
                ],
                vec![
                    InlineKeyboardButton::callback(t!("menu.ops"), "m_ops_center"),
                    InlineKeyboardButton::callback(t!("menu.settings"), "m_settings"),
                ],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_ops_center" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("menu.network_opt"), "m_net_opt"),
                    InlineKeyboardButton::callback(t!("menu.security"), "m_security"),
                ],
                vec![
                    InlineKeyboardButton::callback(t!("menu.sys_cmd"), "m_sys_cmd"),
                    InlineKeyboardButton::callback(t!("menu.log_audit"), "m_log"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_main"),
                    "m_main",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.ops_center"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_settings" => {
            let timeout = ctx.state.session_timeout_secs().await;
            let timeout_label = format!(
                "{}",
                t!("menu.session_timeout", "0" => format_duration_human(timeout))
            );
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("menu.wwps_core_mgmt"), "a_wwps_core_menu"),
                    InlineKeyboardButton::callback(
                        t!("menu.singbox_mgmt_title"),
                        "a_wwps_box_menu",
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.add_task"),
                    "m_sched",
                )],
                vec![
                    InlineKeyboardButton::callback(t!("schedule.geo_update_now"), "a_geo_menu"),
                    InlineKeyboardButton::callback(t!("ops.sys_auto_update"), "a_upgrade"),
                ],
                vec![InlineKeyboardButton::callback(
                    &timeout_label,
                    "m_session_timeout",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.danger_zone"),
                    "m_danger",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_main"),
                    "m_main",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.settings_desc"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_net_opt" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("menu.network_opt"), "m_warp"),
                    InlineKeyboardButton::callback(t!("ops.bbr3_title"), "a_bbr3"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops"),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "🌩 <b>{}</b>\n{}",
                        t!("menu.network_opt"),
                        t!("menu.network_opt_desc")
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_security" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(t!("ops.fw_title"), "a_fw")],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops"),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.security_desc"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_sys_cmd" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback(t!("ops.sys_restart"), "a_sys_reboot"),
                    InlineKeyboardButton::callback(t!("ops.sys_reload_core"), "a_reload"),
                ],
                vec![InlineKeyboardButton::callback(
                    t!("ops.sys_auto_update"),
                    "a_sys_maint",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_ops"),
                    "m_ops_center",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sys_cmd_desc"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_geo_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("schedule.geo_update_now"),
                    "a_geo",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("schedule.geo_auto_sched"),
                    "a_geo_sched_menu",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings"),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.geo_scheduled_title"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "m_mon" => {
            let report = SystemMonitor::get_status_report()
                .await
                .unwrap_or_else(|e| t!("ops.bbr3_fail", "0" => e).into_owned());
            let (wwps_core, wwps_box) = SystemMonitor::get_core_status().await;

            let status_text = format!(
                "{}\n\n🤖 <b>{}</b>: v{}\n\n⚙️ <b>{}</b>:\n- Xray-core: {}\n- Sing-box: {}",
                report,
                t!("menu.monitor"),
                BOT_VERSION,
                t!("menu.settings"),
                if wwps_core { "🟢" } else { "🔴" },
                if wwps_box { "🟢" } else { "🔴" }
            );

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(t!("menu.refresh"), "m_mon")],
                vec![InlineKeyboardButton::callback(t!("menu.back"), "m_main")],
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
                    t!("warp.install_warp"),
                    "a_inst_base",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!(
                            "{}\n\n❌ <b>{}</b>\n\n{}",
                            t!("menu.users"),
                            t!("warp.not_installed"),
                            t!("menu.settings_desc")
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            } else {
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.wwps_core_mgmt"),
                    "m_xray_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.singbox_mgmt_title"),
                    "m_singbox_mgmt",
                )]);
                buttons.push(vec![InlineKeyboardButton::callback(
                    t!("menu.back"),
                    "m_main",
                )]);
                ctx.bot
                    .edit_message_text(
                        ctx.chat_id,
                        ctx.msg_id,
                        format!("{}\n\n{}", t!("menu.users"), t!("menu.settings_desc")),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(InlineKeyboardMarkup::new(buttons))
                    .await?;
            }
        }
        "m_session_timeout" => {
            let current = ctx.state.session_timeout_secs().await;
            let options: Vec<(u64, String)> = vec![
                (5 * 60, format_duration_human(5 * 60)),
                (10 * 60, format_duration_human(10 * 60)),
                (30 * 60, format_duration_human(30 * 60)),
                (60 * 60, format_duration_human(60 * 60)),
                (4 * 3600, format_duration_human(4 * 3600)),
                (12 * 3600, format_duration_human(12 * 3600)),
                (24 * 3600, format_duration_human(24 * 3600)),
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
                t!("menu.back_settings"),
                "m_settings",
            )]);

            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "{}\n\n<b>{}</b>: {}\n\n{}",
                        t!("menu.session_timeout", "0" => format_duration_human(current)),
                        t!("menu.session_timeout"),
                        format_duration_human(current),
                        t!("menu.session_timeout_desc")
                    ),
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
            };
            if let Err(e) = settings.save() {
                log::error!("保存会话设置失败: {}", e);
            }
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("callback.session_timeout_set", "0" => format_duration_human(secs)))
                .await?;

            return Ok(HandlerAction::Redirect("m_session_timeout".to_string()));
        }
        "m_danger" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("destruct.destroy_btn"),
                    "a_destroy_ask",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings"),
                    "m_settings",
                )],
            ]);
            ctx.bot
                .edit_message_text(
                    ctx.chat_id,
                    ctx.msg_id,
                    format!(
                        "{}\n\n{}",
                        t!("menu.danger_zone"),
                        t!("menu.danger_zone_desc")
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("schedule.geo_update_now"),
                    "a_wwps_core_latest",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.log_audit"),
                    "a_wwps_core_tags",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings"),
                    "m_settings",
                )],
            ]);

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.wwps_core_mgmt"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_core_latest" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.upgrade_start"))
                .await?;
            let adapter = ctx.state.adapter.clone();
            let target = TargetId(ctx.chat_id.0.to_string());
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(None, adapter.as_ref(), &target).await
                {
                    let _ = bot_clone
                        .send_message(
                            chat_id_clone,
                            t!("ops.upgrade_fail", "0" => err.to_string()),
                        )
                        .await;
                }
            });
        }
        "a_wwps_core_tags" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("menu.log_audit"))
                .await?;

            let reply = match WwpsCoreUpgradeConfig::from_env()
                .and_then(WwpsCoreUpgradeManager::new)
            {
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
                            t!("menu.back_settings"),
                            "a_wwps_core_menu",
                        )]);
                        ctx.bot
                            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.wwps_core_mgmt"))
                            .reply_markup(InlineKeyboardMarkup::new(buttons))
                            .await
                    }
                    Ok(_) => {
                        ctx.bot
                            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("schedule.geo_stopped"))
                            .await
                    }
                    Err(err) => {
                        ctx.bot
                            .edit_message_text(
                                ctx.chat_id,
                                ctx.msg_id,
                                t!("ops.upgrade_fail", "0" => err.to_string()),
                            )
                            .await
                    }
                },
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("ops.upgrade_fail", "0" => err.to_string()),
                        )
                        .await
                }
            };

            if reply.is_err() {
                let _ = ctx
                    .bot
                    .send_message(ctx.chat_id, t!("ops.geo_fail", "0" => ""))
                    .await;
            }
        }
        d if d.starts_with("wwps_core_tag:") => {
            let tag = d.strip_prefix("wwps_core_tag:").unwrap_or("").to_string();
            if tag.is_empty() {
                ctx.bot
                    .answer_callback_query(ctx.q.id.clone())
                    .text(t!("ops.bbr3_fail", "0" => "").into_owned())
                    .await?;
                return Ok(HandlerAction::Done);
            }

            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.upgrade_start").into_owned())
                .await?;

            let adapter = ctx.state.adapter.clone();
            let target = TargetId(ctx.chat_id.0.to_string());
            let bot_clone = ctx.bot.clone();
            let chat_id_clone = ctx.chat_id;
            tokio::spawn(async move {
                if let Err(err) =
                    WwpsCoreUpgradeManager::run_upgrade(Some(tag), adapter.as_ref(), &target).await
                {
                    let _ = bot_clone
                        .send_message(
                            chat_id_clone,
                            t!("ops.upgrade_fail", "0" => err.to_string()),
                        )
                        .await;
                }
            });
        }
        "a_wwps_box_menu" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("ops.sys_restart"),
                    "a_wwps_box_restart",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.monitor"),
                    "a_wwps_box_status",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("menu.back_settings"),
                    "m_settings",
                )],
            ]);

            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.singbox_mgmt_title"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        "a_wwps_box_restart" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("ops.sys_restart"))
                .await?;

            match SingBoxInstaller::restart_service().await {
                Ok(_) => {
                    ctx.bot
                        .edit_message_text(ctx.chat_id, ctx.msg_id, t!("ops.reload_success"))
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("ops.bbr3_fail", "0" => err.to_string()),
                        )
                        .await?;
                }
            }
        }
        "a_wwps_box_status" => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(t!("menu.monitor"))
                .await?;

            match SingBoxInstaller::status().await {
                Ok(status) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            format!("{}\n\n{}", t!("menu.singbox_mgmt_title"), status),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    ctx.bot
                        .edit_message_text(
                            ctx.chat_id,
                            ctx.msg_id,
                            t!("ops.bbr3_fail", "0" => err.to_string()),
                        )
                        .await?;
                }
            }
        }
        _ => {}
    }
    Ok(HandlerAction::Done)
}
