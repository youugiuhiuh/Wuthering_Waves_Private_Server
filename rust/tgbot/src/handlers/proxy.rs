use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use tgbot::core::types::IpVersion;
use tgbot::logic::config::Proto;
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller};
use tgbot::logic::system::SystemMonitor;

pub async fn show_reality_batch_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    proto: Proto,
) -> ResponseResult<()> {
    let (ip_prefix, title) = match proto {
        Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
        Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        "🌐 IPv4 (0.0.0.0)",
        format!("{}4", ip_prefix),
    )]];

    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("{}6", ip_prefix),
        ));

        if proto == Proto::XHTTP {
            buttons.push(vec![
                InlineKeyboardButton::callback(
                    "🚀 双栈分离 (v6上v4下)",
                    format!("{}s6", ip_prefix),
                ),
                InlineKeyboardButton::callback(
                    "🚀 双栈分离 (v4上v6下)",
                    format!("{}s4", ip_prefix),
                ),
            ]);
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>{} 批量备份 (增强+独立)</b>\n\n✨ <b>自动启用的安全特性:</b>\n• 🎲 随机ShortId (每个配置唯一)\n• 🔄 去重SNI选择 (避免重复)\n• 🏷️ 唯一Tag标识 (基于协议+UUID)\n• 📄 独立配置文件 (不影响原配置)\n\n⬇️ <b>第一步: 请选择网络协议版本:</b>",
            title
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

pub async fn show_reality_qty_prompt(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    ip_version: IpVersion,
    proto: Proto,
) -> ResponseResult<()> {
    let ip_ver_code = match ip_version {
        IpVersion::IPv4 => "4",
        IpVersion::IPv6 => "6",
        IpVersion::SplitStackV6Primary => "s6",
        IpVersion::SplitStackV4Primary => "s4",
    };
    let ip_display = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        IpVersion::SplitStackV6Primary => "双栈分离 (v6上v4下)",
        IpVersion::SplitStackV4Primary => "双栈分离 (v4上v6下)",
    };

    let (exec_prefix, title) = match proto {
        Proto::Vision => ("u_batch_exec:", "Reality"),
        Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
    };

    let buttons = vec![
        vec![
            InlineKeyboardButton::callback("1", format!("{exec_prefix}{ip_ver_code}:1")),
            InlineKeyboardButton::callback("3", format!("{exec_prefix}{ip_ver_code}:3")),
            InlineKeyboardButton::callback("5", format!("{exec_prefix}{ip_ver_code}:5")),
        ],
        vec![
            InlineKeyboardButton::callback("10", format!("{exec_prefix}{ip_ver_code}:10")),
            InlineKeyboardButton::callback("20", format!("{exec_prefix}{ip_ver_code}:20")),
            InlineKeyboardButton::callback("50", format!("{exec_prefix}{ip_ver_code}:50")),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_usr")],
    ];

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>{} 批量备份 (增强+独立)</b>\n\n🌐 网络协议: <b>{}</b>\n\n⬇️ <b>第二步: 请选择生成数量:</b>",
            title, ip_display
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
    Ok(())
}

pub fn trigger_reality_auto_init(bot: Bot, chat_id: ChatId, msg_id: MessageId) {
    tokio::spawn(async move {
        match RealityInstaller::run(bot.clone(), chat_id, msg_id).await {
            Ok(RealityInstallOutcome::AlreadyReady) => {
                let _ = show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision).await;
            }
            Ok(RealityInstallOutcome::Completed) => {
                let _ = show_reality_batch_prompt(&bot, chat_id, msg_id, Proto::Vision).await;
                let _ = bot
                    .send_message(
                        chat_id,
                        "✅ <b>Reality 母版已初始化完成，可继续批量生成。</b>",
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Ok(RealityInstallOutcome::InProgress) => {}
            Err(e) => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!(
                            "❌ <b>Reality 环境初始化失败</b>\n原因: {}\n请尝试运维菜单中【初始化 Reality】或手动执行 install.sh 选项 3。",
                            e
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
    });
}