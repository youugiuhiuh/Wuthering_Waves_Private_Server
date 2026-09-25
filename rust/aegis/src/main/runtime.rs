use std::sync::Arc;

use aegis::common::{InlineButton, Markup, MessageContent, MessageId, TargetId};
use aegis::core::i18n;
use aegis::shared::dispatch_event;
use aegis::shared::types::*;
use anyhow::Result;
use futures_util::StreamExt;
use matrix_sdk::Client as MatrixClient;
use matrix_sdk::Room as MatrixRoom;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::room::encrypted;
use matrix_sdk::ruma::events::room::message::{
    MessageType, Relation, RoomMessageEventContentWithoutRelation,
};
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

/// 读取 `config.enc` 中配置的语言。纯读，无副作用 —— 供需要“在任何本地化调用之前”
/// 提前设定全局 locale 的调用点使用（如 `connect_simplex` 生成欢迎语）。
pub fn configured_lang() -> Option<i18n::Lang> {
    let config_data = std::fs::read(config_dir().join(crate::bootstrap::CONFIG_FILE)).ok()?;
    let encrypted_config: crate::bootstrap::EncryptedConfig =
        serde_json::from_slice(&config_data).ok()?;
    encrypted_config
        .lang
        .as_ref()
        .map(|lang_str| lang_str.parse().unwrap_or(i18n::Lang::Zh))
}

/// 应用 `config.enc` 中记录的语言：设置进程全局 locale、同步 AppState 语言状态，
/// 并执行一次性的系统副作用（时区、apt-daily timer）。
///
/// 副作用只应执行一次，因此本函数在 `main.rs` 中只调用一次；若需要在连接平台前
/// 提前设定 locale，用 [`configured_lang`] + `i18n::set_lang`。
pub async fn apply_configured_language(state: &Arc<AppState>) {
    let Some(lang) = configured_lang() else {
        return;
    };

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

pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    simplex_handle: Option<super::simplex::SimplexHandle>,
    token: Option<String>,
    admin_id: Option<i64>,
) -> Result<(), anyhow::Error> {
    // 语言已在 main.rs 中于连接任何平台之前应用（见 apply_configured_language），
    // 此处不再重复，以免时区/apt timer 副作用被执行两次。

    // ── Matrix 同步循环 ──
    if let Some((client, room, matrix_adapter)) = matrix_handle {
        let target = TargetId(room.room_id().to_string());

        let matrix_target_for_encrypted = target.clone();

        // 组合形态（enable_telegram=true）下 Matrix 只作出站落点，不再注册入站消息
        // handler；纯 `--matrix`（enable_telegram=false）行为保持不变。
        if secondary_handles_inbound(enable_telegram) {
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

            fn extract_thread_root(
                relates_to: &Option<Relation<RoomMessageEventContentWithoutRelation>>,
            ) -> Option<String> {
                relates_to.as_ref().and_then(|r| {
                    if let Relation::Thread(t) = r {
                        Some(t.event_id.to_string())
                    } else {
                        None
                    }
                })
            }

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

                    fn extract_media_info(
                        source: &MediaSource,
                        filename: &str,
                    ) -> (Option<String>, Option<String>) {
                        let fid = match source {
                            MediaSource::Plain(url) => Some(url.to_string()),
                            MediaSource::Encrypted(info) => Some(info.url.to_string()),
                        };
                        let fname = Some(filename.to_string());
                        (fid, fname)
                    }

                    let (file_id, file_name) = match &event.content.msgtype {
                        MessageType::Audio(c) => extract_media_info(&c.source, c.filename()),
                        MessageType::File(c) => extract_media_info(&c.source, c.filename()),
                        MessageType::Image(c) => extract_media_info(&c.source, c.filename()),
                        MessageType::Video(c) => extract_media_info(&c.source, c.filename()),
                        _ => (None, None),
                    };

                    let event = if let Some(ev) = aegis::gateways::matrix::commands::parse_to_event(
                        &text,
                        adapter.clone(),
                        &target,
                        user_id,
                    ) {
                        ev
                    } else {
                        BotEvent::Message(MessageEvent {
                            adapter: adapter.clone(),
                            target: target.clone(),
                            user_id,
                            text: Some(text),
                            file_id,
                            file_name,
                            reply_to_text: None,
                            thread_root: extract_thread_root(&event.content.relates_to),
                        })
                    };
                    let _ = dispatch_event(event, &state).await;
                }
            },
        );
        } else {
            // spec §7 的缓解措施：Matrix 入站被关闭时留下可观测痕迹，否则房间里的
            // 消息会完全静默（用户只会看到「没反应」，无从排查）。
            log::warn!("组合形态下 Matrix 入站交互已关闭：Matrix 仅作敏感内容出站落点");
        }

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

    // TG+SimpleX：分享链接必须在 SimpleX 分支消费 handle 之前取出 —— 而且该分支在
    // `enable_telegram` 时本就跳过自己的启动通知，所以由 TG 分支负责发送。
    let simplex_share_address = crate::simplex_share_link_target(
        enable_telegram,
        simplex_handle.as_ref().and_then(|h| h.address.as_deref()),
    )
    .map(str::to_string);

    // ── SimpleX 网关 ──
    let simplex_enabled = simplex_handle.is_some();
    if let Some(handle) = simplex_handle {
        // SimpleX 是独立平台：调度器与启动通知必须发往 SimpleX 管理员联系人，
        // 而非 Telegram 的 admin_id（后者在纯 SimpleX 部署下为 None）。
        // 此快照**仅用于调度器/启动通知目标**；事件循环里每条入站消息必须实时读
        // `state.simplex_admin_id()`，否则 TOTP 重钉后本进程会把新管理员的命令丢掉。
        let simplex_admin = state.simplex_admin_id();
        // TG + SimpleX 组合下 Telegram 分支已启动唯一一个 scheduler，此处必须跳过，
        // 否则定时通知与启动通知会各发两遍。事件循环不受影响，仍照常接收命令。
        if let Some(admin_contact) = simplex_admin.filter(|_| !enable_telegram) {
            let adapter_for_init = handle.adapter.clone();
            let target_for_init = TargetId(admin_contact.to_string());
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
                        let _ = crate::notify_upgrade_success(&*adapter_for_init, &target_for_init)
                            .await;
                    },
                    async {
                        let _ =
                            crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init)
                                .await;
                    },
                    async {
                        let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
                    },
                );
            });
        } else if !enable_telegram {
            log::error!(
                "SimpleX 未配置 simplex_admin_id，调度器与启动通知不会发送；\
                 请管理员先向 bot 发一条消息，从日志中取得 contactId 后写入配置"
            );
        }

        let state_for_events = state.clone();
        let adapter_for_events = handle.adapter.clone();

        tokio::spawn(async move {
            let mut events = handle.events;
            while let Some(ev) = events.next().await {
                let Ok(ev) = ev else {
                    log::warn!("SimpleX 事件流解析失败，已跳过该事件");
                    continue;
                };
                let items = match ev {
                    simploxide_client::events::Event::NewChatItems(items) => items,
                    // 连接即上报。此前非 NewChatItems 的事件一律被丢弃，于是管理员连上 bot 后
                    // 必须先发一条消息、再从「未授权联系人」告警里反推自己的 contactId ——
                    // 官方设计里 ContactConnected 就是用来处理「正在连接的用户」的。
                    simploxide_client::events::Event::ContactConnected(ev) => {
                        log::info!(
                            "{}",
                            contact_connected_log_line(
                                ev.contact.contact_id,
                                &ev.contact.local_display_name
                            )
                        );
                        continue;
                    }
                    // `Event` 是 `#[non_exhaustive]`：其余事件（ChatItemReaction、
                    // 各类群事件…）仍然不处理。
                    // P2：autoAccept 已关闭，敲门变成 ReceivedContactRequest（不再是
                    // ContactConnected）。此事件**必须在此刻捕获** —— 它携带的
                    // contactRequestId 是 _accept/_reject 的唯一凭据，接受后无法回溯。
                    simploxide_client::events::Event::ReceivedContactRequest(ev) => {
                        let req = &ev.contact_request;
                        log::info!(
                            "SimpleX 待批准联系人请求: contactRequestId={} display_name={:?}",
                            req.contact_request_id,
                            req.local_display_name
                        );
                        if enable_telegram && let Some(tg_admin) = admin_id {
                            let (text, markup) = contact_request_notification(
                                req.contact_request_id,
                                &req.local_display_name,
                            );
                            let target = TargetId(tg_admin.to_string());
                            // 安全通知强制走 primary（TG）：文本含攻击者可控的显示名，
                            // 若走 send_message 会被 RoutingAdapter 的敏感内容匹配改投
                            // 到 SimpleX，使 TG 端收不到提示、按钮失效。
                            if let Err(e) = state_for_events
                                .adapter
                                .send_message_primary(
                                    &target,
                                    MessageContent {
                                        text,
                                        markup: Some(markup),
                                    },
                                )
                                .await
                            {
                                log::error!("向 Telegram 推送待批准联系人请求失败: {e}");
                            }
                        }
                        continue;
                    }
                    _ => continue,
                };

                let mapped = aegis::gateways::simplex::map_new_chat_items(&items.chat_items);
                if mapped.is_empty() {
                    // ChatInfo 的 untagged `Undocumented` 回退会静默丢弃未来版本/
                    // 畸形载荷 —— 留下痕迹以便排查“消息没反应”。
                    log::warn!(
                        "SimpleX NewChatItems 未映射出任何消息（chatItems={}），\
                         可能是群聊/非文本内容或协议版本不匹配",
                        items.chat_items.len()
                    );
                    continue;
                }

                for msg in mapped {
                    // 实时读：TOTP 重钉后（自愈）本进程必须立即把新身份当管理员，
                    // 否则新管理员的 `/menu` 等命令会被下面的门禁① 静默丢弃。
                    let is_admin = state_for_events.simplex_admin_id() == Some(msg.contact_id);
                    if let SimplexInbound::Drop =
                        classify_simplex_inbound(enable_telegram, is_admin, msg.text.as_deref())
                    {
                        let reason = if secondary_handles_inbound(enable_telegram) {
                            "未授权联系人（仅 6 位登录码放行）"
                        } else {
                            "组合形态下 SimpleX 只作出站落点"
                        };
                        log::warn!("SimpleX 入站已丢弃 contactId={}：{reason}", msg.contact_id);
                        continue;
                    }

                    // 文本命令必须走与 Matrix 相同的解析器：dispatch_event 的
                    // Message 分支不解析斜杠命令，只有 parse_to_event 会产出
                    // BotEvent::Command / BotEvent::Callback。
                    let text = msg.text.as_deref().unwrap_or("");
                    let event = if let Some(ev) = aegis::gateways::matrix::commands::parse_to_event(
                        text,
                        adapter_for_events.clone(),
                        &msg.target,
                        msg.user_id,
                    ) {
                        ev
                    } else {
                        BotEvent::Message(MessageEvent {
                            adapter: adapter_for_events.clone(),
                            target: msg.target.clone(),
                            user_id: msg.user_id,
                            text: msg.text.clone(),
                            file_id: msg.file_id.clone(),
                            file_name: msg.file_name.clone(),
                            reply_to_text: None,
                            thread_root: None,
                        })
                    };
                    let _ = dispatch_event(event, &state_for_events).await;
                }
            }
            log::warn!("SimpleX 事件流已结束");
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
            let _ = dispatch_event(
                BotEvent::Command(CommandEvent {
                    adapter: state.adapter.clone(),
                    target: TargetId(msg.chat.id.0.to_string()),
                    user_id: msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0),
                    command: teloxide_to_bot(cmd),
                }),
                &state,
            )
            .await;
            Ok(())
        }

        async fn handle_message(
            _bot: Bot,
            msg: Message,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let user_id = msg.from.as_ref().map(|f| f.id.0 as i64).unwrap_or(0);
            let _ = dispatch_event(
                BotEvent::Message(MessageEvent {
                    adapter: state.adapter.clone(),
                    target: TargetId(msg.chat.id.0.to_string()),
                    user_id,
                    text: msg.text().map(|s| s.to_string()),
                    file_id: msg.document().map(|d| d.file.id.to_string()).or_else(|| {
                        msg.photo()
                            .and_then(|p| p.last().map(|ph| ph.file.id.to_string()))
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
            _bot: Bot,
            q: CallbackQuery,
            state: Arc<AppState>,
        ) -> Result<(), teloxide::RequestError> {
            let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
            let msg_id = q.message.as_ref().map(|m| m.id()).unwrap_or_default();
            let _ = dispatch_event(
                BotEvent::Callback(CallbackEvent {
                    adapter: state.adapter.clone(),
                    target: TargetId(chat_id.0.to_string()),
                    user_id: q.from.id.0.to_string(),
                    msg_id: MessageId(msg_id.0.to_string()),
                    data: q.data.clone().unwrap_or_default(),
                    callback_id: q.id.to_string(),
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

        let adapter_for_init = state.adapter.clone();
        let target_for_init = TargetId(admin_id.unwrap_or(0).to_string());
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
                async {
                    // TG+SimpleX：先给管理员递上 SimpleX 分享链接，否则用户无从连接。
                    if let Some(address) = simplex_share_address.as_deref() {
                        match crate::notify_simplex_share_link(
                            &*adapter_for_init,
                            &target_for_init,
                            address,
                        )
                        .await
                        {
                            Ok(()) => log::info!("已向 TG 推送 SimpleX 分享链接"),
                            Err(e) => log::warn!("向 TG 推送 SimpleX 分享链接失败: {e}"),
                        }
                    }
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
        let target_for_init = TargetId(admin_id.unwrap_or(0).to_string());
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

    // ── SimpleX-only: 保活（事件循环在 spawn 中运行）──
    if simplex_enabled && !enable_telegram && !enable_matrix {
        let token = CancellationToken::new();
        let token_clone = token.clone();

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            log::info!("收到关闭信号，正在优雅关闭 SimpleX...");
            token.cancel();
        });

        token_clone.cancelled().await;
    }

    Ok(())
}

/// 组装 `ContactConnected` 的上报行。
///
/// 显示名用 `{:?}` 而非 `{}`：地址是公开的、自动接受的，任何人都能连接并自设显示名，
/// 不转义控制字符就能用换行伪造日志行（日志注入）。
///
/// 单独抽成纯函数仅为可测：事件循环跑在 `tokio::spawn` 里、依赖活的 WebSocket，
/// 「分支是否存在」这类缺失型回归无法用单测覆盖（P1 的日志器缺陷同属此类）。
fn contact_connected_log_line(contact_id: i64, display_name: &str) -> String {
    format!("SimpleX 新联系人连接: contactId={contact_id} display_name={display_name:?}")
}

/// 组装「有人敲门」的 TG 通知。回调 data 携带的是 `contactRequestId`
/// （`_accept` / `_reject` 需要的 ID），不是 contactId。
///
/// 显示名由陌生人自设，且会经 Telegram `ParseMode::Html` 渲染。`{:?}` 只转义
/// 引号/反斜杠/控制字符（堵换行注入），**不转义 `<>&`**；因此再叠一层 HTML 转义。
fn contact_request_notification(contact_request_id: i64, display_name: &str) -> (String, Markup) {
    let text = rust_i18n::t!(
        "simplex.contact_request",
        "0" => escape_display_name(display_name),
        "1" => contact_request_id.to_string()
    )
    .into_owned();
    let markup = Markup {
        buttons: vec![vec![
            InlineButton {
                text: rust_i18n::t!("simplex.approve").into_owned(),
                data: format!("sx_approve:{contact_request_id}"),
            },
            InlineButton {
                text: rust_i18n::t!("simplex.reject").into_owned(),
                data: format!("sx_reject:{contact_request_id}"),
            },
        ]],
    };
    (text, markup)
}

/// 显示名是攻击者可控字符串，且会经 Telegram HTML parse_mode 渲染。
/// 先 Debug 转义（引号/反斜杠/控制字符/换行），再 HTML 转义（`<>&`），两处都堵住。
fn escape_display_name(name: &str) -> String {
    crate::utils::escape_html(&format!("{name:?}"))
}

/// 组合形态下 secondary 平台（SimpleX / Matrix）是否处理入站交互。
/// primary（TG）已启用时，secondary 只作出站落点。
pub(crate) fn secondary_handles_inbound(primary_enabled: bool) -> bool {
    !primary_enabled
}

/// SimpleX 入站消息的处置。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SimplexInbound {
    /// 交给 dispatch_event（命令 / 文本 / 登录码）。
    Dispatch,
    /// 丢弃（仅记日志）。
    Drop,
}

/// TG 启用时 SimpleX 一律不交互；纯 SimpleX 时沿用既有门禁
/// （管理员全放行，非管理员只放行 6 位纯数字码 = TOTP 自愈入口）。
pub(crate) fn classify_simplex_inbound(
    enable_telegram: bool,
    is_admin: bool,
    text: Option<&str>,
) -> SimplexInbound {
    if !secondary_handles_inbound(enable_telegram) {
        return SimplexInbound::Drop;
    }
    // 管理员的消息全放行；非管理员**只有 6 位纯数字码**放行 —— 那是登录尝试，
    // 也是新 contactId 证明自己的唯一途径（TOTP 自愈的前提）。
    //
    // 与 `shared::dispatch::is_totp_code` 保持同样的判据（6 位 ASCII 数字）。
    // 此处无法复用那个私有函数（跨 crate 边界：runtime.rs 在 bin，dispatch 在 lib），
    // 因此判据写在这里；两处不一致会让码在门口被丢，是本模块最该盯的回归点。
    if is_admin || text.is_some_and(|t| t.len() == 6 && t.chars().all(|c| c.is_ascii_digit())) {
        SimplexInbound::Dispatch
    } else {
        SimplexInbound::Drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_connected_log_line_reports_contact_id_and_name() {
        let line = contact_connected_log_line(3, "alice");
        assert!(line.contains("contactId=3"), "{line}");
        assert!(line.contains("display_name=\"alice\""), "{line}");
    }

    /// 地址是公开的、自动接受的，任何人都能连接并自设显示名。显示名里的换行/
    /// 控制字符必须被转义，否则陌生人能伪造日志行（日志注入）。
    #[test]
    fn contact_connected_log_line_escapes_control_chars_in_display_name() {
        let line = contact_connected_log_line(7, "evil\n[WARN] SimpleX 未授权联系人 contactId=1");
        assert!(
            !line.contains('\n'),
            "显示名里的换行必须被转义，实际: {line}"
        );
        assert!(line.contains("\\n"), "换行应以转义形式出现，实际: {line}");
    }

    #[test]
    fn contact_request_notification_has_approve_and_reject_buttons() {
        let (text, markup) = contact_request_notification(5, "alice");
        assert!(text.contains('5'), "{text}");
        assert!(text.contains("alice"), "{text}");
        let datas: Vec<&str> = markup
            .buttons
            .iter()
            .flatten()
            .map(|b| b.data.as_str())
            .collect();
        assert_eq!(datas, vec!["sx_approve:5", "sx_reject:5"]);
    }

    /// 显示名由陌生人自设：换行必须以字面 `\n` 出现，且 `<`/`>`/`&` 必须被
    /// HTML 转义（通知经 Telegram HTML parse_mode 渲染，否则可注入链接/标签）。
    #[test]
    fn contact_request_notification_escapes_display_name() {
        let (text, _) = contact_request_notification(7, "evil\nINJECTED");
        assert!(
            text.contains(r#"evil\nINJECTED"#),
            "显示名的换行应以字面 \\n 出现: {text}"
        );
        assert!(
            !text.contains("evil\nINJECTED"),
            "显示名不得注入真实换行: {text}"
        );

        let (html, _) = contact_request_notification(7, "<a href='x'>pwn</a>");
        assert!(!html.contains('<'), "不得残留未转义的 '<': {html}");
        assert!(html.contains("&lt;a"), "'<' 应被 HTML 转义: {html}");
    }

    #[test]
    fn classify_simplex_inbound_dispatches_totp_code_from_unknown_contact() {
        assert_eq!(
            classify_simplex_inbound(false, false, Some("123456")),
            SimplexInbound::Dispatch,
            "非管理员发来的 6 位码必须放行，否则新 contactId 无法自愈"
        );
    }

    #[test]
    fn classify_simplex_inbound_drops_ordinary_text_from_unknown_contact() {
        assert_eq!(
            classify_simplex_inbound(false, false, Some("/menu")),
            SimplexInbound::Drop
        );
        assert_eq!(
            classify_simplex_inbound(false, false, Some("hello")),
            SimplexInbound::Drop
        );
        assert_eq!(
            classify_simplex_inbound(false, false, None),
            SimplexInbound::Drop
        );
    }

    #[test]
    fn classify_simplex_inbound_dispatches_everything_from_admin() {
        assert_eq!(
            classify_simplex_inbound(false, true, Some("/menu")),
            SimplexInbound::Dispatch
        );
        assert_eq!(
            classify_simplex_inbound(false, true, Some("hello")),
            SimplexInbound::Dispatch
        );
        assert_eq!(
            classify_simplex_inbound(false, true, None),
            SimplexInbound::Dispatch
        );
    }

    #[test]
    fn classify_simplex_inbound_does_not_treat_near_miss_codes_as_login() {
        for text in [Some("12345"), Some("1234567"), Some("12345a")] {
            assert_eq!(
                classify_simplex_inbound(false, false, text),
                SimplexInbound::Drop,
                "近失码不得当作登录: {text:?}"
            );
        }
    }

    #[test]
    fn secondary_handles_inbound_is_false_when_primary_enabled() {
        assert!(!secondary_handles_inbound(true));
    }

    #[test]
    fn secondary_handles_inbound_is_true_when_primary_disabled() {
        assert!(secondary_handles_inbound(false));
    }

    /// TG 形态下 SimpleX 一律不交互：管理员命令、6 位码、空文本都丢弃。
    #[test]
    fn classify_simplex_inbound_drops_everything_when_telegram_enabled() {
        assert_eq!(
            classify_simplex_inbound(true, true, Some("/menu")),
            SimplexInbound::Drop
        );
        assert_eq!(
            classify_simplex_inbound(true, false, Some("123456")),
            SimplexInbound::Drop
        );
        assert_eq!(
            classify_simplex_inbound(true, true, None),
            SimplexInbound::Drop
        );
    }

    /// 纯 SimpleX 形态：沿用既有门禁判据，逐条不回归。
    #[test]
    fn classify_simplex_inbound_pure_simplex_keeps_old_rules() {
        assert_eq!(
            classify_simplex_inbound(false, true, Some("/menu")),
            SimplexInbound::Dispatch
        );
        assert_eq!(
            classify_simplex_inbound(false, false, Some("123456")),
            SimplexInbound::Dispatch
        );
        for text in [
            Some("/menu"),
            Some("hello"),
            None,
            Some("12345"),
            Some("1234567"),
            Some("12345a"),
        ] {
            assert_eq!(
                classify_simplex_inbound(false, false, text),
                SimplexInbound::Drop,
                "非管理员近失码/普通文本必须丢弃: {text:?}"
            );
        }
    }
}
