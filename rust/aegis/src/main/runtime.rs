use std::sync::Arc;

use aegis::adapters::common::{BotAdapter, TargetId};
use aegis::core::i18n;
use matrix_sdk::Client as MatrixClient;
use matrix_sdk::Room as MatrixRoom;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

use crate::app::state::AppState;
use crate::bootstrap::config_dir;
use crate::handlers::{callback, message};
use crate::{Command, handle_command};

pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    token: String,
    admin_id: i64,
) -> Result<(), anyhow::Error> {
    // Initialize i18n language from config
    if let Ok(config_data) = std::fs::read(config_dir().join(crate::bootstrap::CONFIG_FILE)) {
        if let Ok(encrypted_config) =
            serde_json::from_slice::<crate::bootstrap::EncryptedConfig>(&config_data)
        {
            if let Some(ref lang_str) = encrypted_config.lang {
                let lang = lang_str.parse().unwrap_or(i18n::Lang::Zh);
                i18n::set_lang(lang);
                state.set_lang(lang).await;
                state.mark_lang_configured().await;
                i18n::mark_lang_configured();

                let tz = i18n::lang_to_timezone(lang);
                match tokio::process::Command::new("timedatectl")
                    .args(["set-timezone", tz])
                    .output()
                    .await
                {
                    Ok(o) if !o.status.success() => {
                        log::warn!("设置系统时区 {} 失败: exit {:?}", tz, o.status.code());
                    }
                    Err(e) => log::warn!("设置系统时区 {} 失败: {}", tz, e),
                    _ => {}
                }

                if let Err(e) =
                    aegis::core::system::operations::Operations::set_apt_daily_timer().await
                {
                    log::warn!("覆盖 apt-daily timer 失败: {}", e);
                }
            }
        }
    }

    // ── Matrix 同步循环 ──
    if let Some((client, room, matrix_adapter)) = matrix_handle {
        let target = TargetId(room.room_id().to_string());

        fn parse_user_id(s: &str) -> i64 {
            s.trim_start_matches('@')
                .split(':')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        }

        let matrix_state = state.clone();
        let matrix_adapter_sync = matrix_adapter;
        let matrix_target = target.clone();

        client.add_event_handler(
            move |event: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent,
                  room: MatrixRoom,
                  _client: MatrixClient| {
                let state = matrix_state.clone();
                let adapter = matrix_adapter_sync.clone();
                let target = matrix_target.clone();
                async move {
                    if room.room_id().as_str() != target.0.as_str() {
                        return;
                    }
                    let user_id = parse_user_id(event.sender.as_str());
                    if !state.is_admin_user(user_id) {
                        return;
                    }
                    let text = event.content.body().trim().to_string();

                    if crate::looks_like_totp_code(&text) && !state.is_authorized(user_id).await {
                        let _ = crate::process_auth_code(&state, &target, user_id, &text).await;
                        return;
                    }

                    let cmd = aegis::adapters::matrix::commands::parse(&text);
                    if !matches!(cmd, aegis::adapters::matrix::commands::Command::Auth { .. }) {
                        let _ = crate::matrix_handlers::dispatch(&cmd, &*adapter, &target, &state)
                            .await;
                    }
                }
            },
        );

        tokio::spawn(async move {
            if let Err(e) = client
                .sync(matrix_sdk::config::SyncSettings::default())
                .await
            {
                log::error!("Matrix sync error: {}", e);
            }
        });
    }

    // ── Telegram Dispatcher ──
    if enable_telegram {
        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint(handle_command),
            )
            .branch(Update::filter_message().endpoint(message::handle_message))
            .branch(Update::filter_callback_query().endpoint(callback::handle_callback));

        let adapter_for_init = state.adapter.clone();
        let target_for_init = TargetId(admin_id.to_string());
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            let _ = crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
            let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init).await;
            let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
        });

        Dispatcher::builder(Bot::new(&token), handler)
            .dependencies(dptree::deps![state.clone()])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    }

    // ── Matrix-only: 后台初始化 + 保活 ──
    if enable_matrix && !enable_telegram {
        let adapter_for_init = state.adapter.clone();
        let target_for_init = TargetId(admin_id.to_string());
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            let _ = crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
            let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init).await;
            let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
        });

        // 保活 — matrix sync runs in background via spawn above
        let () = std::future::pending().await;
    }

    Ok(())
}
