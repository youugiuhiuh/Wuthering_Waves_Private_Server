//! Declarative Configuration Examples / 声明式配置示例
//!
//! This file demonstrates correct and incorrect usage of declarative paradigm
//! in rust/tgbot codebase.

use serde_json::json;

// ============================================================================
// CORRECT EXAMPLES / 正确示例
// ============================================================================

/// ✅ Declarative: JSON 配置生成 - 配置即代码
/// 场景: 为 sing-box 生成完整的入站配置
pub fn build_singbox_config(inbounds: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "log": {
            "level": "warning",
            "output": "/var/log/sing-box.log"
        },
        "dns": {
            "servers": [
                {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
                {"tag": "local", "type": "local"}
            ]
        },
        "route": {
            "default_domain_resolver": "dns"
        },
        "inbounds": inbounds,
        "outbounds": [
            {"type": "direct", "tag": "direct"},
            {"type": "block", "tag": "block"}
        ]
    })
}

/// ✅ Declarative: Hysteria2 Inbound 配置生成
pub fn build_hysteria2_inbound(
    tag: &str,
    port: u16,
    password: &str,
    sni: &str,
    obfs_password: Option<&str>,
) -> serde_json::Value {
    let mut inbound = json!({
        "type": "hysteria2",
        "tag": tag,
        "listen": "::",
        "listen_port": port,
        "users": [
            {"password": password}
        ],
        "tls": {
            "enabled": true,
            "server_name": sni,
            "alpn": ["h3"],
            "key_path": "/etc/wwps/wwps-box/certs/tls.key",
            "certificate_path": "/etc/wwps/wwps-box/certs/tls.cer"
        }
    });

    // 添加混淆配置（如果启用）
    if let Some(obfs_pwd) = obfs_password {
        inbound["obfs"] = json!({
            "type": "salamander",
            "password": obfs_pwd
        });
    }

    inbound
}

/// ✅ Declarative: UI 键盘构建 - 声明式 API
/// 场景: 构建自定义时间选择键盘
pub fn build_time_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        // 第一行：小时
        vec![
            InlineKeyboardButton::callback("00:00", "s_custom_set:time:00:00"),
            InlineKeyboardButton::callback("01:00", "s_custom_set:time:01:00"),
            InlineKeyboardButton::callback("02:00", "s_custom_set:time:02:00"),
            InlineKeyboardButton::callback("03:00", "s_custom_set:time:03:00"),
            InlineKeyboardButton::callback("04:00", "s_custom_set:time:04:00"),
        ],
        // 第二行：更多小时
        vec![
            InlineKeyboardButton::callback("05:00", "s_custom_set:time:05:00"),
            InlineKeyboardButton::callback("06:00", "s_custom_set:time:06:00"),
            InlineKeyboardButton::callback("07:00", "s_custom_set:time:07:00"),
            InlineKeyboardButton::callback("08:00", "s_custom_set:time:08:00"),
            InlineKeyboardButton::callback("09:00", "s_custom_set:time:09:00"),
        ],
        // 返回按钮
        vec![
            InlineKeyboardButton::callback("🔙 返回", "s_custom"),
        ],
    ])
}

/// ✅ Declarative: 路由匹配 - 模式匹配
/// 场景: 处理回调查询
pub async fn handle_callback(bot: &Bot, callback: CallbackQuery) -> Result<()> {
    let data = callback.data.unwrap_or_default();

    // 使用 match 进行声明式路由
    match data.as_str() {
        // 主菜单
        "m_main" => show_main_menu(bot, &callback).await?,
        "m_ops_center" => show_ops_center(bot, &callback).await?,

        // 安装菜单
        "m_install" => show_install_menu(bot, &callback).await?,
        "m_install_xray" => handle_install_xray(bot, &callback).await?,
        "m_install_singbox" => handle_install_singbox(bot, &callback).await?,

        // 卸载菜单
        "m_uninstall" => show_uninstall_menu(bot, &callback).await?,

        // 升级菜单
        "m_upgrade" => show_upgrade_menu(bot, &callback).await?,

        // Sing-box Hysteria2 处理
        d if d.starts_with("sb_h2_exec:") => {
            handle_singbox_hy2_create(bot, &callback, d).await?
        }
        d if d.starts_with("sb_h2_del:") => {
            handle_singbox_hy2_delete(bot, &callback, d).await?
        }

        // Sing-box TUIC 处理
        d if d.starts_with("sb_tuic_exec:") => {
            handle_singbox_tuic_create(bot, &callback, d).await?
        }

        // 默认处理
        _ => {}
    }

    Ok(())
}

/// ✅ Declarative: 静态配置 - 数据即代码
/// 场景: 网络优化配置常量
const NETWORK_OPTIMIZE_CONF: &str = r#"fs.file-max = 1000000
fs.inotify.max_user_instances = 8192
fs.inotify.max_user_watches = 524288
vm.swappiness = 60
vm.dirty_ratio = 15
vm.dirty_background_ratio = 5
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
"#;

/// ✅ Declarative: 使用 const 声明静态配置
const DEFAULT_TIMEOUT_SECS: u64 = 300;

const SUPPORTED_PROTOCOLS: &[&str] = &[
    "vless",
    "vmess",
    "trojan",
    "shadowsocks",
    "hysteria2",
    "tuic",
];

/// ✅ Declarative: 使用 match 映射枚举
fn protocol_to_display_name(protocol: &str) -> &'static str {
    match protocol {
        "vless" => "VLESS",
        "vmess" => "VMess",
        "trojan" => "Trojan",
        "shadowsocks" => "Shadowsocks",
        "hysteria2" => "Hysteria2",
        "tuic" => "TUIC",
        _ => "Unknown",
    }
}

// ============================================================================
// ANTI-PATTERNS / 反模式
// ============================================================================

/// ❌ Imperative: 不要用 if-else 链替代模式匹配
pub async fn handle_callback_bad(bot: &Bot, callback: CallbackQuery) -> Result<()> {
    let data = callback.data.unwrap_or_default();

    // ❌ 冗长的 if-else 链
    if data == "m_main" {
        show_main_menu(bot, &callback).await?;
    } else if data == "m_ops_center" {
        show_ops_center(bot, &callback).await?;
    } else if data == "m_install" {
        show_install_menu(bot, &callback).await?;
    } else if data == "m_uninstall" {
        show_uninstall_menu(bot, &callback).await?;
    } else if data.starts_with("sb_h2_exec:") {
        handle_singbox_hy2_create(bot, &callback, &data).await?;
    } else if data.starts_with("sb_h2_del:") {
        handle_singbox_hy2_delete(bot, &callback, &data).await?;
    }
    // ... 更多 else-if

    Ok(())
}

/// ❌ Imperative: 不要手动构建复杂 JSON
fn build_config_bad() -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // ❌ 手动添加每个字段
    map.insert("log".to_string(), serde_json::json!({
        "level": "warning",
        "output": "/var/log/sing-box.log"
    }));

    let mut dns_map = serde_json::Map::new();
    dns_map.insert("servers".to_string(), serde_json::json!([]));
    map.insert("dns".to_string(), serde_json::Value::Object(dns_map));

    // ... 更多手动构建

    Value::Object(map)
}

/// ❌ Declarative: 不要在不适当的场景使用声明式
/// 例如：复杂业务逻辑不适合用 json! 宏

// ============================================================================
// HELPER FUNCTIONS / 辅助函数
// ============================================================================

struct Bot;
struct CallbackQuery {
    data: Option<String>,
    message: Option<()>,
    id: String,
    from: (),
}

struct InlineKeyboardMarkup {
    _data: (),
}

impl InlineKeyboardMarkup {
    fn new(rows: Vec<Vec<InlineKeyboardButton>>) -> Self {
        Self { _data: () }
    }
}

struct InlineKeyboardButton {
    text: String,
    callback_data: String,
}

impl InlineKeyboardButton {
    fn callback(text: &str, data: &str) -> Self {
        Self {
            text: text.to_string(),
            callback_data: data.to_string(),
        }
    }
}

async fn show_main_menu(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn show_ops_center(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn show_install_menu(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn show_uninstall_menu(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn show_upgrade_menu(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn handle_install_xray(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn handle_install_singbox(bot: &Bot, callback: &CallbackQuery) -> Result<()> {
    Ok(())
}

async fn handle_singbox_hy2_create(bot: &Bot, callback: &CallbackQuery, data: &str) -> Result<()> {
    Ok(())
}

async fn handle_singbox_hy2_delete(bot: &Bot, callback: &CallbackQuery, data: &str) -> Result<()> {
    Ok(())
}

async fn handle_singbox_tuic_create(bot: &Bot, callback: &CallbackQuery, data: &str) -> Result<()> {
    Ok(())
}

use anyhow::Result;
use serde_json::Value;