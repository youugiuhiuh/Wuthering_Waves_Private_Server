use std::sync::Arc;

use aegis::app::interaction::{
    ActorId, BusinessCommand, BusinessResult, ConversationId, Origin, PlatformId,
};
use aegis::app::output::BusinessOutput;
use aegis::common::{BotAdapter, MessageId, TargetId};
use aegis::core::i18n;
use aegis::shared::dispatch_event;
use aegis::shared::types::*;
use anyhow::Context;
use anyhow::Result;
use matrix_sdk::Client as MatrixClient;
use matrix_sdk::Room as MatrixRoom;
use matrix_sdk::ruma::events::room::encrypted;
#[cfg(feature = "telegram")]
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
#[cfg(feature = "telegram")]
use teloxide::prelude::*;
#[cfg(feature = "telegram")]
use teloxide::types::{CallbackQuery, ChatId, Message};
#[cfg(feature = "telegram")]
use teloxide::utils::command::BotCommands;
use tokio_util::sync::CancellationToken;

use crate::bootstrap::config_dir;
use aegis::app::service::ApplicationService;
use aegis::app::state::AppState;
use aegis::gateways::matrix::commands::command_to_business_input;
use aegis::gateways::matrix::presenter::MatrixPresenter;
#[cfg(feature = "telegram")]
use aegis::gateways::telegram::TelegramAdapter;
#[cfg(feature = "telegram")]
use aegis::gateways::telegram::mapping;
#[cfg(feature = "telegram")]
use aegis::gateways::telegram::presenter::TelegramPresenter;
#[cfg(feature = "telegram")]
use aegis::shared::handlers::menu::{AdapterOutput, send_main_menu};

#[cfg(feature = "telegram")]
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum TeloxideCommand {
    #[command(description = "Show help")]
    Help,
    #[command(description = "Start bot")]
    Start,
    #[command(description = "Show admin menu")]
    Menu,
    #[command(description = "Verify TOTP code")]
    Auth(String),
    #[command(description = "Set destruct verification file")]
    SetSecurityFile,
}

#[cfg(feature = "telegram")]
#[allow(dead_code)]
pub(crate) async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(TeloxideCommand::bot_commands())
        .await
        .map_err(|e| anyhow::anyhow!("无法向 Telegram 注册主命令: {e}"))?;
    Ok(())
}

#[cfg_attr(not(feature = "telegram"), allow(unused_variables))]
pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    discord_raw: Option<super::discord::DiscordRawHandle>,
    token: Option<String>,
    admin_id: Option<i64>,
) -> Result<(), anyhow::Error> {
    // Initialize i18n language from config
    if let Ok(config_data) = std::fs::read(config_dir().join(crate::bootstrap::CONFIG_FILE))
        && let Ok(encrypted_config) =
            serde_json::from_slice::<crate::bootstrap::EncryptedConfig>(&config_data)
        && let Some(ref lang_str) = encrypted_config.lang
    {
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

        if let Err(e) = aegis::core::system::operations::Operations::set_apt_daily_timer().await {
            log::warn!("覆盖 apt-daily timer 失败: {}", e);
        }
    }

    // ── Discord 网关 ──
    if let Some(raw) = discord_raw {
        let adapter_for_init = raw.adapter.clone();
        let target_for_init = TargetId(raw.admin_channel.to_string());
        let scheduler_reporter = aegis::shared::reporters::SendMessageReporter::new(
            adapter_for_init.clone(),
            target_for_init.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) =
                aegis::core::system::scheduler::start_scheduler(Arc::new(scheduler_reporter)).await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            tokio::join!(
                async {
                    let _ =
                        crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
                },
                async {
                    let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init)
                        .await;
                },
                async {
                    let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
                },
            );
        });

        let (mut client, _, _) = super::discord::build_handle(raw, state.clone())
            .await
            .context("构建 Discord 客户端失败")?;

        // Discord-only: keep process alive via CancellationToken
        if !enable_telegram && !enable_matrix {
            let token = CancellationToken::new();
            let token_clone = token.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                log::info!("收到关闭信号，正在优雅关闭...");
                token.cancel();
            });
            tokio::spawn(async move {
                if let Err(e) = client.start().await {
                    log::error!("Discord 网关错误: {}", e);
                }
            });
            token_clone.cancelled().await;
        } else {
            tokio::spawn(async move {
                if let Err(e) = client.start().await {
                    log::error!("Discord 网关错误: {}", e);
                }
            });
        }
    }

    // ── Matrix 同步循环 ──
    let matrix_adapter_for_notify = matrix_handle.as_ref().map(|(_, _, a)| a.clone());
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
        let matrix_target_for_encrypted = matrix_target.clone();

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

                    if let Some(bot_cmd) =
                        aegis::gateways::matrix::commands::parse_to_bot_command(&text)
                    {
                        let cmd = match bot_cmd {
                            BotCommand::Help => {
                                Some(aegis::gateways::matrix::commands::Command::Help)
                            }
                            BotCommand::Menu => {
                                Some(aegis::gateways::matrix::commands::Command::Menu)
                            }
                            BotCommand::Auth { code } => {
                                Some(aegis::gateways::matrix::commands::Command::Auth { code })
                            }
                            BotCommand::Start | BotCommand::SetSecurityFile => None,
                        };
                        if let Some(cmd) = cmd
                            && let Some(input) = command_to_business_input(cmd, user_id, &target)
                        {
                            let presenter = MatrixPresenter::new(room);
                            let _ = ApplicationService.handle(&input, &state, &presenter).await;
                            return;
                        }
                    }

                    let event = if let Some(ev) = aegis::gateways::matrix::commands::parse_to_event(
                        &text,
                        adapter.clone(),
                        &target,
                        user_id,
                    ) {
                        ev
                    } else {
                        return;
                    };
                    let _ = dispatch_event(event, &state).await;
                }
            },
        );

        client.add_event_handler(
            move |event: encrypted::SyncRoomEncryptedEvent,
                  room: MatrixRoom,
                  _client: MatrixClient| {
                let target = matrix_target_for_encrypted.clone();
                async move {
                    if room.room_id().as_str() != target.0.as_str() {
                        return;
                    }
                    log::debug!(
                        "Encrypted event in thread room from {}: {:?}",
                        event.sender(),
                        event.event_id()
                    );
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
    #[cfg(feature = "telegram")]
    if enable_telegram {
        async fn handle_command(
            bot: Bot,
            msg: Message,
            cmd: TeloxideCommand,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            match cmd {
                TeloxideCommand::Auth(code) => {
                    let adapter: Arc<dyn BotAdapter> = Arc::new(TelegramAdapter::new(bot.clone()));
                    let target = TargetId(msg.chat.id.0.to_string());
                    let user_id = msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0);
                    let output: Arc<dyn BusinessOutput> =
                        Arc::new(AdapterOutput::new(adapter.clone(), target.clone()));
                    let origin = Origin {
                        platform: PlatformId::Telegram,
                        actor_id: ActorId::new(user_id.to_string()).unwrap(),
                        conversation_id: ConversationId::new(target.0.clone()).unwrap(),
                    };
                    let _ = dispatch_event(
                        BotEvent::Command(CommandEvent {
                            output,
                            origin,
                            target,
                            user_id,
                            command: BotCommand::Auth { code },
                        }),
                        &state,
                    )
                    .await;
                }
                TeloxideCommand::Menu => {
                    let target = TargetId(msg.chat.id.0.to_string());
                    if let Ok(input) = mapping::command_input(&msg, BusinessCommand::Menu) {
                        let presenter = TelegramPresenter::new(bot.clone());
                        if let Ok(result) =
                            ApplicationService.handle(&input, &state, &presenter).await
                            && matches!(result, BusinessResult::Ok)
                        {
                            let adapter: Arc<dyn BotAdapter> =
                                Arc::new(TelegramAdapter::new(bot.clone()));
                            let output = AdapterOutput::new(adapter, target.clone());
                            let conversation_id = ConversationId::new(target.0).unwrap();
                            let _ = send_main_menu(&output, &conversation_id).await;
                        }
                    }
                }
                _ => {
                    let command = match cmd {
                        TeloxideCommand::Help => BusinessCommand::Help,
                        TeloxideCommand::Start => BusinessCommand::Start,
                        TeloxideCommand::SetSecurityFile => BusinessCommand::SetSecurityFile,
                        TeloxideCommand::Auth(_) | TeloxideCommand::Menu => unreachable!(),
                    };
                    if let Ok(input) = mapping::command_input(&msg, command) {
                        let presenter = TelegramPresenter::new(bot.clone());
                        let _ = ApplicationService.handle(&input, &state, &presenter).await;
                    }
                }
            }
            Ok(())
        }

        async fn handle_message(
            bot: Bot,
            msg: Message,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            if let Some(text) = msg.text().map(|s| s.to_string())
                && let Ok(ref input) = mapping::text_input(&msg, text.clone())
            {
                let presenter = TelegramPresenter::new(bot.clone());
                if let Ok(BusinessResult::Message(_)) =
                    ApplicationService.handle(input, &state, &presenter).await
                {
                    return Ok(());
                }
            }
            let user_id = msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0);
            let adapter: Arc<dyn BotAdapter> = Arc::new(TelegramAdapter::new(bot.clone()));
            let target = TargetId(msg.chat.id.0.to_string());
            let output: Arc<dyn BusinessOutput> =
                Arc::new(AdapterOutput::new(adapter.clone(), target.clone()));
            let origin = Origin {
                platform: PlatformId::Telegram,
                actor_id: ActorId::new(user_id.to_string()).unwrap(),
                conversation_id: ConversationId::new(target.0.clone()).unwrap(),
            };
            let _ = dispatch_event(
                BotEvent::Message(MessageEvent {
                    output,
                    origin,
                    target,
                    user_id,
                    text: msg.text().map(|s| s.to_string()),
                    file_id: msg.document().map(|d| d.file.id.clone()).or_else(|| {
                        msg.photo()
                            .and_then(|p| p.last().map(|ph| ph.file.id.clone()))
                    }),
                    file_name: msg
                        .document()
                        .and_then(|d| d.file_name.clone())
                        .or_else(|| {
                            msg.photo()
                                .map(|_| rust_i18n::t!("destruct.image_label").to_string())
                        }),
                    reply_to_text: msg
                        .reply_to_message()
                        .and_then(|r| r.text().map(|s| s.to_string())),
                    thread_root: None,
                }),
                &state,
            )
            .await;
            Ok(())
        }

        async fn handle_callback(
            bot: Bot,
            q: CallbackQuery,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();
            let adapter: Arc<dyn BotAdapter> = Arc::new(TelegramAdapter::new(bot.clone()));
            let target = TargetId(chat_id.0.to_string());
            let user_id_str = q.from.id.0.to_string();
            let output: Arc<dyn BusinessOutput> =
                Arc::new(AdapterOutput::new(adapter.clone(), target.clone()));
            let origin = Origin {
                platform: PlatformId::Telegram,
                actor_id: ActorId::new(user_id_str.clone()).unwrap(),
                conversation_id: ConversationId::new(target.0.clone()).unwrap(),
            };
            let _ = dispatch_event(
                BotEvent::Callback(CallbackEvent {
                    output,
                    origin,
                    target,
                    user_id: user_id_str,
                    msg_id: MessageId(msg_id.0.to_string()),
                    data: q.data.clone().unwrap_or_default(),
                    callback_id: q.id.clone(),
                    session_timeout_secs: state.session_timeout_secs().await,
                }),
                &state,
            )
            .await;
            Ok(())
        }

        let bot = Bot::new(token.as_deref().unwrap());

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<TeloxideCommand>()
                    .endpoint(handle_command),
            )
            .branch(Update::filter_message().endpoint(handle_message))
            .branch(Update::filter_callback_query().endpoint(handle_callback));

        let tg_adapter: Arc<dyn BotAdapter> = Arc::new(TelegramAdapter::new(bot.clone()));
        let adapter_for_init = tg_adapter.clone();
        let target_for_init = TargetId(admin_id.unwrap_or(0).to_string());
        let scheduler_reporter = aegis::shared::reporters::SendMessageReporter::new(
            adapter_for_init.clone(),
            target_for_init.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) =
                aegis::core::system::scheduler::start_scheduler(Arc::new(scheduler_reporter)).await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            tokio::join!(
                async {
                    let _ =
                        crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
                },
                async {
                    let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init)
                        .await;
                },
                async {
                    let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
                },
            );
        });

        Dispatcher::builder(bot.clone(), handler)
            .dependencies(dptree::deps![state.clone()])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    }

    // ── Matrix-only: 后台初始化 + 保活 ──
    if enable_matrix && !enable_telegram {
        let adapter_for_init = matrix_adapter_for_notify.unwrap();
        let target_for_init = TargetId(admin_id.unwrap_or(0).to_string());
        let scheduler_reporter = aegis::shared::reporters::SendMessageReporter::new(
            adapter_for_init.clone(),
            target_for_init.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) =
                aegis::core::system::scheduler::start_scheduler(Arc::new(scheduler_reporter)).await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            tokio::join!(
                async {
                    let _ =
                        crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
                },
                async {
                    let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init)
                        .await;
                },
                async {
                    let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
                },
            );
        });

        // 保活 — matrix sync runs in background via spawn above
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // 处理 SIGTERM/SIGINT 触发优雅关闭
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            log::info!("收到关闭信号，正在优雅关闭...");
            token.cancel();
        });

        token_clone.cancelled().await;
    }

    Ok(())
}
