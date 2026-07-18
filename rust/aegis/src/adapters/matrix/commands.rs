use std::sync::Arc;

use aegis::adapters::common::{BotAdapter, MessageId, Principal, TargetId};
use aegis::shared::types::{BotCommand, BotEvent, CallbackEvent, CommandEvent};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Auth { code: String },
    Help,
    Status,
    Menu,
    Xray(XraySubCommand),
    Singbox(SingboxSubCommand),
    Ops(OpsSubCommand),
    Destruct,
    Schedule(ScheduleSubCommand),
    Warp(WarpSubCommand),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum XraySubCommand {
    Status,
    Add { proto: String, count: usize },
    Del { proto: Option<String> },
    PqStatus,
    PqGen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SingboxSubCommand {
    Status,
    Add { proto: String, count: usize },
    Del,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpsSubCommand {
    Reload,
    Upgrade,
    Maintenance,
    Bbr3,
    Geo,
    Fw,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleSubCommand {
    List,
    Add,
    Del { index: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarpSubCommand {
    Status,
    Install,
    Uninstall,
}

pub fn parse(text: &str) -> Command {
    let trimmed = text.trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Command::Unknown(String::new());
    }

    match parts[0].to_lowercase().as_str() {
        "auth" => {
            if parts.len() >= 2 {
                Command::Auth {
                    code: parts[1].to_string(),
                }
            } else {
                Command::Unknown("auth <code> - 需要 6 位验证码".to_string())
            }
        }
        "help" | "h" => Command::Help,
        "status" => Command::Status,
        "menu" => Command::Menu,
        "xray" => parse_xray(&parts[1..]),
        "sb" | "singbox" => parse_singbox(&parts[1..]),
        "ops" => parse_ops(&parts[1..]),
        "destruct" => Command::Destruct,
        "sched" | "schedule" => parse_schedule(&parts[1..]),
        "warp" => parse_warp(&parts[1..]),
        other => Command::Unknown(format!("未知命令: {}，输入 help 查看可用命令", other)),
    }
}

fn parse_xray(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Xray(XraySubCommand::Status),
        Some("add") => {
            let proto = args.get(1).map(|s| s.to_string()).unwrap_or_default();
            let count = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            Command::Xray(XraySubCommand::Add { proto, count })
        }
        Some("del") | Some("delete") => Command::Xray(XraySubCommand::Del {
            proto: args.get(1).map(|s| s.to_string()),
        }),
        Some("pq") => match args.get(1).map(|s| s.to_lowercase()).as_deref() {
            Some("status") => Command::Xray(XraySubCommand::PqStatus),
            Some("gen" | "generate") => Command::Xray(XraySubCommand::PqGen),
            _ => Command::Xray(XraySubCommand::PqStatus),
        },
        _ => Command::Unknown(format!("未知 xray 子命令: {:?}", args)),
    }
}

fn parse_singbox(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Singbox(SingboxSubCommand::Status),
        Some("add") => {
            let proto = args.get(1).map(|s| s.to_string()).unwrap_or_default();
            let count = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            Command::Singbox(SingboxSubCommand::Add { proto, count })
        }
        Some("del") | Some("delete") => Command::Singbox(SingboxSubCommand::Del),
        _ => Command::Unknown(format!("未知 singbox 子命令: {:?}", args)),
    }
}

fn parse_ops(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        Some("reload") => Command::Ops(OpsSubCommand::Reload),
        Some("upgrade") => Command::Ops(OpsSubCommand::Upgrade),
        Some("maintenance") | Some("maint") => Command::Ops(OpsSubCommand::Maintenance),
        Some("bbr3") => Command::Ops(OpsSubCommand::Bbr3),
        Some("geo") => Command::Ops(OpsSubCommand::Geo),
        Some("fw") | Some("firewall") => Command::Ops(OpsSubCommand::Fw),
        _ => Command::Unknown(
            "可用 ops 子命令: reload, upgrade, maintenance, bbr3, geo, fw".to_string(),
        ),
    }
}

fn parse_schedule(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("list") => Command::Schedule(ScheduleSubCommand::List),
        Some("add") => Command::Schedule(ScheduleSubCommand::Add),
        Some("del") | Some("delete") => {
            let index = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            Command::Schedule(ScheduleSubCommand::Del { index })
        }
        _ => Command::Unknown("可用 schedule 子命令: list, add, del <index>".to_string()),
    }
}

pub fn parse_to_bot_command(text: &str) -> Option<BotCommand> {
    let t = text.trim();
    if t == "/help" || t == "/h" {
        return Some(BotCommand::Help);
    }
    if t == "/start" {
        return Some(BotCommand::Start);
    }
    if t == "/menu" {
        return Some(BotCommand::Menu);
    }
    if t == "/setsecurityfile" {
        return Some(BotCommand::SetSecurityFile);
    }
    if let Some(code) = t.strip_prefix("/auth ") {
        return Some(BotCommand::Auth {
            code: code.trim().to_string(),
        });
    }
    None
}

pub fn parse_to_event(
    text: &str,
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    principal: &Principal,
) -> Option<BotEvent> {
    let text = text.trim();

    // Try basic BotCommand commands first
    if let Some(cmd) = parse_to_bot_command(text) {
        return Some(BotEvent::Command(CommandEvent {
            adapter,
            target: target.clone(),
            principal: principal.clone(),
            command: cmd,
        }));
    }

    let text_lower = text.to_lowercase();
    let target = target.clone();
    let principal = principal.clone();

    let event = |data: &str| -> BotEvent {
        BotEvent::Callback(CallbackEvent {
            adapter,
            target,
            principal,
            msg_id: MessageId("0".into()),
            data: data.to_string(),
            callback_id: format!("synth:{}", data),
            session_timeout_secs: 600,
        })
    };

    // ops subcommands — 1:1 mapping to callback data
    if let Some(data) = text_lower.strip_prefix("ops ") {
        return Some(match data {
            "reload" => event("a_reload"),
            "upgrade" => event("a_upgrade"),
            "fw" | "firewall" => event("a_fw"),
            "geo" => event("a_geo"),
            "bbr3" => event("a_bbr3"),
            "maintenance" | "tune" => event("a_tune"),
            _ => return None,
        });
    }

    // warp subcommands — 1:1 mapping
    if let Some(data) = text_lower.strip_prefix("warp ") {
        return Some(match data {
            "status" => event("a_warp_status"),
            "install" => event("a_inst_warp"),
            "uninstall" => event("a_warp_uninstall"),
            _ => return None,
        });
    }

    // destruct — start the flow
    if text_lower == "destruct" {
        return Some(event("a_destroy_ask"));
    }

    // xray — parse subcommands or fallback to menu
    if text_lower.starts_with("xray ") || text_lower == "xray" {
        // xray add <proto> <count> <ip> → batch exec
        if let Some(params) = text_lower.strip_prefix("xray add ")
            && !params.is_empty()
        {
            return Some(event(&format!("u_batch_exec:{}", params)));
        }
        // xray delete/del <name> → cfg_del
        // Check "delete" before "del" to avoid matching extra chars
        if let Some(name) = text_lower.strip_prefix("xray delete ") {
            return Some(event(&format!("cfg_del:{}", name)));
        }
        if let Some(name) = text_lower.strip_prefix("xray del ") {
            return Some(event(&format!("cfg_del:{}", name)));
        }
        // xray routing → routing menu
        if text_lower == "xray routing" {
            return Some(event("m_routing"));
        }
        // xray pq status → pq management
        if text_lower == "xray pq status" {
            return Some(event("m_pq_mgmt"));
        }
        // fallback: show xray menu
        return Some(event("m_xray_mgmt"));
    }

    // singbox install shortcut and subcommands
    if text_lower == "singbox install" {
        return Some(event("sb_install"));
    }
    if let Some(cmd) = text_lower.strip_prefix("sb ") {
        return Some(match cmd {
            "install" => event("sb_install"),
            cmd if cmd.starts_with("add h2 ") => {
                let params = cmd.strip_prefix("add h2 ").unwrap();
                let parts: Vec<&str> = params.split_whitespace().collect();
                if parts.len() >= 2 {
                    event(&format!("sb_h2_ip:{},{}", parts[0], parts[1]))
                } else {
                    event("m_singbox_mgmt")
                }
            }
            cmd if cmd.starts_with("add tu ") => {
                let params = cmd.strip_prefix("add tu ").unwrap();
                let parts: Vec<&str> = params.split_whitespace().collect();
                if parts.len() >= 2 {
                    event(&format!("sb_tu_ip:{},{}", parts[0], parts[1]))
                } else {
                    event("m_singbox_mgmt")
                }
            }
            cmd if cmd.starts_with("del ") || cmd.starts_with("delete ") => {
                let name = cmd
                    .strip_prefix("del ")
                    .or_else(|| cmd.strip_prefix("delete "))
                    .unwrap()
                    .trim();
                event(&format!("sb_del_cfg:{}", name))
            }
            _ => event("m_singbox_mgmt"),
        });
    }
    if text_lower == "singbox" || text_lower == "sb" {
        return Some(event("m_singbox_mgmt"));
    }

    // schedule — show menu or handle subcommands
    if text_lower.starts_with("schedule ")
        || text_lower == "schedule"
        || text_lower.starts_with("sched ")
        || text_lower == "sched"
    {
        // schedule add <template>
        if let Some(template) = text_lower
            .strip_prefix("schedule add ")
            .or_else(|| text_lower.strip_prefix("sched add "))
        {
            return Some(event(&format!("s_add:{}", template)));
        }
        // schedule del <idx>
        if let Some(idx) = text_lower
            .strip_prefix("schedule del ")
            .or_else(|| text_lower.strip_prefix("sched del "))
            .or_else(|| text_lower.strip_prefix("schedule delete "))
            .or_else(|| text_lower.strip_prefix("sched delete "))
        {
            return Some(event(&format!("s_del:{}", idx)));
        }
        // fallback: menu
        return Some(event("m_sched"));
    }

    None
}

fn parse_warp(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Warp(WarpSubCommand::Status),
        Some("install") => Command::Warp(WarpSubCommand::Install),
        Some("uninstall") | Some("remove") => Command::Warp(WarpSubCommand::Uninstall),
        _ => Command::Unknown("可用 warp 子命令: status, install, uninstall".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_command() {
        assert_eq!(
            parse("auth 123456"),
            Command::Auth {
                code: "123456".to_string()
            }
        );
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse("help"), Command::Help);
    }

    #[test]
    fn parse_xray_status() {
        assert_eq!(parse("xray status"), Command::Xray(XraySubCommand::Status));
    }

    #[test]
    fn parse_xray_add_with_count() {
        assert_eq!(
            parse("xray add reality 5"),
            Command::Xray(XraySubCommand::Add {
                proto: "reality".to_string(),
                count: 5
            })
        );
    }

    #[test]
    fn parse_ops_reload() {
        assert_eq!(parse("ops reload"), Command::Ops(OpsSubCommand::Reload));
    }

    #[test]
    fn parse_unknown() {
        assert!(matches!(parse("blah"), Command::Unknown(_)));
    }

    #[test]
    fn parse_empty() {
        assert!(matches!(parse(""), Command::Unknown(_)));
    }
}

#[cfg(test)]
mod parse_to_event_tests {
    use super::*;
    use aegis::adapters::common::{
        BotAdapter, MockBotAdapter, Platform, PlatformCapabilities, Principal, TargetId,
    };
    use std::sync::Arc;

    fn test_adapter() -> Arc<dyn BotAdapter> {
        let mut m = MockBotAdapter::new();
        m.expect_platform().returning(|| Platform::Matrix);
        m.expect_capabilities().returning(|| PlatformCapabilities {
            can_edit_message: false,
            can_delete_message: false,
            has_inline_keyboard: false,
            has_slash_commands: false,
            has_file_transfer: false,
        });
        Arc::new(m)
    }

    fn test_principal() -> Principal {
        Principal::telegram(42)
    }

    #[test]
    fn parse_help_returns_command() {
        let result = parse_to_event(
            "/help",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Command(CommandEvent {
                command: BotCommand::Help,
                ..
            }))
        ));
    }

    #[test]
    fn parse_ops_reload_returns_callback() {
        let result = parse_to_event(
            "ops reload",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_reload"
        ));
    }

    #[test]
    fn parse_warp_status_returns_callback() {
        let result = parse_to_event(
            "warp status",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_warp_status"
        ));
    }

    #[test]
    fn parse_destruct_returns_callback() {
        let result = parse_to_event(
            "destruct",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_destroy_ask"
        ));
    }

    #[test]
    fn parse_xray_returns_menu_callback() {
        let result = parse_to_event(
            "xray status",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_xray_mgmt"
        ));
    }

    #[test]
    fn parse_schedule_returns_menu_callback() {
        let result = parse_to_event(
            "schedule list",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_sched"
        ));
    }

    #[test]
    fn parse_unknown_text_returns_none() {
        let result = parse_to_event(
            "some random text",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn parse_xray_add_returns_batch_exec() {
        let result = parse_to_event(
            "xray add reality 5 1.2.3.4",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "u_batch_exec:reality 5 1.2.3.4"
        ));
    }

    #[test]
    fn parse_xray_del_returns_cfg_del() {
        let result = parse_to_event(
            "xray del myconfig",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "cfg_del:myconfig"
        ));
    }

    #[test]
    fn parse_xray_delete_returns_cfg_del() {
        let result = parse_to_event(
            "xray delete myconfig",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "cfg_del:myconfig"
        ));
    }

    #[test]
    fn parse_xray_routing_returns_m_routing() {
        let result = parse_to_event(
            "xray routing",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_routing"
        ));
    }

    #[test]
    fn parse_xray_pq_status_returns_m_pq_mgmt() {
        let result = parse_to_event(
            "xray pq status",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_pq_mgmt"
        ));
    }

    #[test]
    fn parse_xray_status_falls_back_to_menu() {
        let result = parse_to_event(
            "xray status",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_xray_mgmt"
        ));
    }

    #[test]
    fn parse_sb_install_returns_sb_install() {
        let result = parse_to_event(
            "sb install",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "sb_install"
        ));
    }

    #[test]
    fn parse_sb_add_h2_returns_sb_h2_ip() {
        let result = parse_to_event(
            "sb add h2 example.com 5",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "sb_h2_ip:example.com,5"
        ));
    }

    #[test]
    fn parse_sb_add_tu_returns_sb_tu_ip() {
        let result = parse_to_event(
            "sb add tu example.com 3",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "sb_tu_ip:example.com,3"
        ));
    }

    #[test]
    fn parse_sb_del_returns_sb_del_cfg() {
        let result = parse_to_event(
            "sb del myconfig",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "sb_del_cfg:myconfig"
        ));
    }

    #[test]
    fn parse_sb_unknown_returns_menu() {
        let result = parse_to_event(
            "sb status",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_singbox_mgmt"
        ));
    }

    #[test]
    fn parse_singbox_bare_returns_menu() {
        let result = parse_to_event(
            "singbox",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_singbox_mgmt"
        ));
    }

    #[test]
    fn parse_schedule_add_returns_s_add() {
        let result = parse_to_event(
            "schedule add mytemplate",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "s_add:mytemplate"
        ));
    }

    #[test]
    fn parse_sched_add_returns_s_add() {
        let result = parse_to_event(
            "sched add mytemplate",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "s_add:mytemplate"
        ));
    }

    #[test]
    fn parse_schedule_del_returns_s_del() {
        let result = parse_to_event(
            "schedule del 3",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "s_del:3"
        ));
    }

    #[test]
    fn parse_sched_del_returns_s_del() {
        let result = parse_to_event(
            "sched del 3",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "s_del:3"
        ));
    }

    #[test]
    fn parse_schedule_delete_returns_s_del() {
        let result = parse_to_event(
            "schedule delete 3",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "s_del:3"
        ));
    }

    #[test]
    fn parse_schedule_list_returns_menu() {
        let result = parse_to_event(
            "schedule list",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_sched"
        ));
    }

    #[test]
    fn parse_schedule_bare_returns_menu() {
        let result = parse_to_event(
            "schedule",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_sched"
        ));
    }

    #[test]
    fn parse_sched_bare_returns_menu() {
        let result = parse_to_event(
            "sched",
            test_adapter(),
            &TargetId("!r:localhost".into()),
            &test_principal(),
        );
        assert!(matches!(
            result,
            Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_sched"
        ));
    }
}
