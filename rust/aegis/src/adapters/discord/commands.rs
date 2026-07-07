use serenity::all::{CommandOptionType, CreateCommand, CreateCommandOption};

pub fn all_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("auth")
            .description("TOTP 验证")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "code", "6 位 TOTP 验证码")
                    .required(true)
                    .min_length(6)
                    .max_length(6),
            ),
        CreateCommand::new("status").description("系统状态报告"),
        CreateCommand::new("menu").description("主菜单"),
        CreateCommand::new("xray")
            .description("Xray 管理")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "查看 Xray 状态",
            ))
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "add", "添加配置")
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "proto",
                            "协议 (reality/vision)",
                        )
                        .required(true),
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::Integer,
                            "count",
                            "数量 (默认 1)",
                        )
                        .min_int_value(1)
                        .max_int_value(100),
                    ),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "del", "删除配置")
                    .add_sub_option(CreateCommandOption::new(
                        CommandOptionType::String,
                        "proto",
                        "协议 (可选，不指定则删除全部)",
                    )),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "pq",
                "PQ 密钥管理",
            )),
        CreateCommand::new("singbox")
            .description("SingBox 管理")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "查看 SingBox 状态",
            ))
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "add", "添加配置")
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "proto",
                            "协议 (hy2/tuic)",
                        )
                        .required(true),
                    )
                    .add_sub_option(CreateCommandOption::new(
                        CommandOptionType::Integer,
                        "count",
                        "数量 (默认 1)",
                    )),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "del",
                "删除所有配置",
            )),
        CreateCommand::new("ops")
            .description("运维操作")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "reload",
                "重载核心",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "upgrade",
                "自更新",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "maintenance",
                "系统维护",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "bbr3",
                "BBR3 安装",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "geo",
                "更新 GeoData",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "fw",
                "防火墙加固",
            )),
        CreateCommand::new("warp")
            .description("WARP 管理")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "WARP 状态",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "install",
                "安装 WARP",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "uninstall",
                "卸载 WARP",
            )),
        CreateCommand::new("schedule")
            .description("定时任务管理")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "list",
                "列出定时任务",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "add",
                "添加定时任务",
            ))
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "del", "删除定时任务")
                    .add_sub_option(
                        CreateCommandOption::new(CommandOptionType::Integer, "index", "任务序号")
                            .required(true),
                    ),
            ),
    ]
}
