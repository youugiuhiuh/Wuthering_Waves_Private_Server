use std::sync::Arc;

use aegis::app::state::AppState;
use aegis::adapters::common::{BotAdapter, TargetId};
use aegis::adapters::matrix::commands::*;

use aegis::handlers::context::HandlerContext;

pub async fn dispatch(
    cmd: &Command,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let data = match cmd {
        Command::Help | Command::Menu => "m_main".to_string(),
        Command::Status => "m_status".to_string(),
        Command::Xray(XraySubCommand::Status) => "a_xray".to_string(),
        Command::Xray(XraySubCommand::Add { .. }) => "a_xray_add".to_string(),
        Command::Xray(XraySubCommand::Del { .. }) => "a_xray_del".to_string(),
        Command::Xray(XraySubCommand::PqStatus | XraySubCommand::PqGen) => "m_pq_mgmt".to_string(),
        Command::Singbox(SingboxSubCommand::Status) => "m_singbox_mgmt".to_string(),
        Command::Singbox(SingboxSubCommand::Add { .. }) => "sb_add".to_string(),
        Command::Singbox(SingboxSubCommand::Del) => "sb_del".to_string(),
        Command::Ops(OpsSubCommand::Reload) => "a_reload".to_string(),
        Command::Ops(OpsSubCommand::Upgrade) => "a_upgrade".to_string(),
        Command::Ops(OpsSubCommand::Maintenance) => "a_sys_maint".to_string(),
        Command::Ops(OpsSubCommand::Bbr3) => "a_bbr3".to_string(),
        Command::Ops(OpsSubCommand::Geo) => "a_geo".to_string(),
        Command::Ops(OpsSubCommand::Fw) => "a_fw".to_string(),
        Command::Warp(WarpSubCommand::Status) => "a_warp_status".to_string(),
        Command::Warp(WarpSubCommand::Install) => "a_warp_install".to_string(),
        Command::Warp(WarpSubCommand::Uninstall) => "a_warp_uninstall".to_string(),
        Command::Schedule(ScheduleSubCommand::List) => "s_list".to_string(),
        Command::Schedule(ScheduleSubCommand::Add) => "s_add".to_string(),
        Command::Schedule(ScheduleSubCommand::Del { .. }) => "s_del".to_string(),
        Command::Destruct => "a_destroy_ask".to_string(),
        Command::Unknown(msg) => {
            adapter
                .send_message(
                    target,
                    aegis::adapters::common::MessageContent {
                        text: msg.clone(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(());
        }
        Command::Auth { .. } => return Ok(()),
    };

    let user_id = target.0.parse::<i64>().unwrap_or(0);
    let hctx = HandlerContext {
        adapter,
        target: target.clone(),
        state,
        user_id,
        data,
        msg_id: None,
    };

    aegis::handlers::dispatch::dispatch(&hctx).await?;
    Ok(())
}
