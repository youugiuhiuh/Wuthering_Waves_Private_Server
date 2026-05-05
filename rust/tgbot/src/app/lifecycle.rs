use crate::logic::maintenance::{BBR3_PENDING_FLAG_FILE, MaintenanceManager};
use crate::logic::system::SystemMonitor;
use crate::logic::upgrade::UPGRADE_FLAG_FILE;
use anyhow::Result;
use std::fs;
use std::path::Path;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

pub async fn notify_online(bot: &Bot, admin_id: i64) -> Result<()> {
    let ip = match SystemMonitor::get_public_ip().await {
        Ok(ip) => ip,
        Err(err) => {
            log::warn!("获取公网 IPv4 失败: {}", err);
            "Unavailable".to_string()
        }
    };

    let masked_ip = if ip.contains('.') {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 {
            format!("{}.{}.*.*", parts[0], parts[1])
        } else {
            ip.clone()
        }
    } else {
        ip.clone()
    };

    let sys_info = "Linux";

    let msg = format!(
        "🤖 **Bot 已上线**\n\n🌍 IP: `{}`\n💻 系统: {}",
        masked_ip, sys_info
    );

    let _ = bot
        .send_message(ChatId(admin_id), msg)
        .parse_mode(ParseMode::MarkdownV2)
        .await;
    Ok(())
}

pub async fn notify_upgrade_success(bot: &Bot, admin_id: i64) -> Result<()> {
    let flag_path = Path::new(UPGRADE_FLAG_FILE);
    if !flag_path.exists() {
        return Ok(());
    }

    let version_raw = fs::read_to_string(flag_path).unwrap_or_default();
    let version = version_raw.trim();
    if let Err(e) = fs::remove_file(flag_path) {
        eprintln!("[WARN] 无法删除升级标记文件: {}", e);
    }

    let message = if version.is_empty() {
        "✅ Bot 已完成自更新。".to_string()
    } else {
        format!("✅ Bot 已成功更新至 {}。", version)
    };

    bot.send_message(ChatId(admin_id), message).await?;
    Ok(())
}

pub async fn notify_bbr3_reboot_result(bot: &Bot, admin_id: i64) -> Result<()> {
    let flag_path = Path::new(BBR3_PENDING_FLAG_FILE);
    if !flag_path.exists() {
        return Ok(());
    }

    let info = MaintenanceManager::collect_bbr3_runtime_info().await;

    if let Err(e) = fs::remove_file(flag_path) {
        eprintln!("[WARN] 无法删除 BBR3 标记文件: {}", e);
    }

    let kernel_hint = if info.has_xanmod_kernel { "是" } else { "否" };
    let proc_hint = if info.has_xanmod_proc_version {
        "是"
    } else {
        "否"
    };

    let message = format!(
        "✅ <b>BBR3 重启后校验结果</b>\n\n<code>uname -r</code>\n<code>{}</code>\n\n<code>sysctl net.ipv4.tcp_congestion_control</code>\n<code>net.ipv4.tcp_congestion_control = {}</code>\n\n<code>cat /proc/version</code>\n<code>{}</code>\n\n内核名包含 XanMod: <b>{}</b>\n/proc/version 包含 XanMod: <b>{}</b>",
        info.uname_r, info.tcp_congestion_control, info.proc_version, kernel_hint, proc_hint
    );

    bot.send_message(ChatId(admin_id), message)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}