use std::sync::Arc;

use aegis::adapters::common::{Attachment, MessageId, Platform, Principal, TargetId};
use aegis::core::i18n;
use aegis::shared::boundary::{EventContext, handle_dispatch_result};
use aegis::shared::dispatch_event;
use aegis::shared::types::*;
use anyhow::Context;
use anyhow::Result;
use matrix_sdk::Client as MatrixClient;
use matrix_sdk::Room as MatrixRoom;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::room::message::MessageType;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, Message};
use teloxide::utils::command::BotCommands;
use tokio_util::sync::CancellationToken;

use crate::bootstrap::config_dir;
use aegis::app::state::AppState;

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

pub(crate) async fn register_bot_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(TeloxideCommand::bot_commands())
        .await
        .map_err(|e| anyhow::anyhow!("无法向 Telegram 注册主命令: {e}"))?;
    Ok(())
}

fn teloxide_to_bot(cmd: TeloxideCommand) -> BotCommand {
    match cmd {
        TeloxideCommand::Help => BotCommand::Help,
        TeloxideCommand::Start => BotCommand::Start,
        TeloxideCommand::Menu => BotCommand::Menu,
        TeloxideCommand::Auth(code) => BotCommand::Auth { code },
        TeloxideCommand::SetSecurityFile => BotCommand::SetSecurityFile,
    }
}

fn telegram_declared_size(size: u32) -> Option<u64> {
    (size != u32::MAX).then_some(u64::from(size))
}

pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    discord_raw: Option<super::discord::DiscordRawHandle>,
    token: String,
    admin_id: i64,
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
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            tokio::join!(
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
    if let Some((client, room, matrix_adapter)) = matrix_handle {
        let target = TargetId(room.room_id().to_string());

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
                    let text = event.content.body().trim().to_string();

                    fn matrix_attachment(
                        source: &MediaSource,
                        filename: &str,
                        declared_size: Option<u64>,
                    ) -> Attachment {
                        let file_id = match source {
                            MediaSource::Plain(url) => url.to_string(),
                            MediaSource::Encrypted(info) => info.url.to_string(),
                        };
                        Attachment {
                            file_id,
                            file_name: Some(filename.to_string()),
                            declared_size,
                        }
                    }

                    let attachment = match &event.content.msgtype {
                        MessageType::Audio(c) => Some(matrix_attachment(
                            &c.source,
                            c.filename(),
                            c.info.as_ref().and_then(|info| info.size.map(u64::from)),
                        )),
                        MessageType::File(c) => Some(matrix_attachment(
                            &c.source,
                            c.filename(),
                            c.info.as_ref().and_then(|info| info.size.map(u64::from)),
                        )),
                        MessageType::Image(c) => Some(matrix_attachment(
                            &c.source,
                            c.filename(),
                            c.info.as_ref().and_then(|info| info.size.map(u64::from)),
                        )),
                        MessageType::Video(c) => Some(matrix_attachment(
                            &c.source,
                            c.filename(),
                            c.info.as_ref().and_then(|info| info.size.map(u64::from)),
                        )),
                        _ => None,
                    };

                    let principal = Principal::matrix(event.sender.as_str()).unwrap_or_else(|_| {
                        Principal::new(Platform::Matrix, "unknown:localhost").unwrap()
                    });
                    if !state.is_admin_user(&principal) {
                        return;
                    }
                    let event = if let Some(ev) = aegis::adapters::matrix::commands::parse_to_event(
                        &text,
                        adapter.clone(),
                        &target,
                        &principal,
                    ) {
                        ev
                    } else {
                        BotEvent::Message(MessageEvent {
                            adapter: adapter.clone(),
                            target: target.clone(),
                            principal,
                            text: Some(text),
                            attachment,
                            reply_to_text: None,
                        })
                    };
                    let ctx = EventContext::from_event(&event);
                    let result = dispatch_event(event, &state).await;
                    handle_dispatch_result(&state, &ctx, result).await;
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
        async fn handle_command(
            _bot: Bot,
            msg: Message,
            cmd: TeloxideCommand,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let id = msg.from.as_ref().map(|f| f.id.0).unwrap_or(0);
            let event = BotEvent::Command(CommandEvent {
                adapter: state.adapter.clone(),
                target: TargetId(msg.chat.id.0.to_string()),
                principal: Principal::telegram(id),
                command: teloxide_to_bot(cmd),
            });
            let ctx = EventContext::from_event(&event);
            let result = dispatch_event(event, &state).await;
            handle_dispatch_result(&state, &ctx, result).await;
            Ok(())
        }

        async fn handle_message(
            _bot: Bot,
            msg: Message,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let id = msg.from.as_ref().map(|f| f.id.0).unwrap_or(0);
            let attachment = msg
                .document()
                .map(|document| Attachment {
                    file_id: document.file.id.clone(),
                    file_name: document.file_name.clone(),
                    declared_size: telegram_declared_size(document.file.size),
                })
                .or_else(|| {
                    msg.photo().and_then(|photos| {
                        photos.last().map(|photo| Attachment {
                            file_id: photo.file.id.clone(),
                            file_name: Some(rust_i18n::t!("destruct.image_label").to_string()),
                            declared_size: telegram_declared_size(photo.file.size),
                        })
                    })
                });
            let event = BotEvent::Message(MessageEvent {
                adapter: state.adapter.clone(),
                target: TargetId(msg.chat.id.0.to_string()),
                principal: Principal::telegram(id),
                text: msg.text().map(|s| s.to_string()),
                attachment,
                reply_to_text: msg
                    .reply_to_message()
                    .and_then(|r| r.text().map(|s| s.to_string())),
            });
            let ctx = EventContext::from_event(&event);
            let result = dispatch_event(event, &state).await;
            handle_dispatch_result(&state, &ctx, result).await;
            Ok(())
        }

        async fn handle_callback(
            _bot: Bot,
            q: CallbackQuery,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();
            let event = BotEvent::Callback(CallbackEvent {
                adapter: state.adapter.clone(),
                target: TargetId(chat_id.0.to_string()),
                principal: Principal::telegram(q.from.id.0),
                msg_id: MessageId(msg_id.0.to_string()),
                data: q.data.clone().unwrap_or_default(),
                callback_id: q.id.clone(),
                session_timeout_secs: state.session_timeout_secs().await,
            });
            let ctx = EventContext::from_event(&event);
            let result = dispatch_event(event, &state).await;
            handle_dispatch_result(&state, &ctx, result).await;
            Ok(())
        }

        let bot = Bot::new(&token);

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<TeloxideCommand>()
                    .endpoint(handle_command),
            )
            .branch(Update::filter_message().endpoint(handle_message))
            .branch(Update::filter_callback_query().endpoint(handle_callback));

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
            tokio::join!(
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
            tokio::join!(
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

#[cfg(test)]
mod attachment_metadata_tests {
    use super::telegram_declared_size;

    #[test]
    fn telegram_missing_size_fallback_stays_untrusted() {
        assert_eq!(telegram_declared_size(u32::MAX), None);
        assert_eq!(telegram_declared_size(42), Some(42));
    }
}
