use aegis::shared::types::BotCommand;

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
