pub(crate) mod callback;
pub mod context;
pub(crate) mod destruct_flow_wrapper;
pub mod log;
pub mod menu;
pub(crate) mod message;
pub mod ops;
pub mod schedule;
pub mod singbox;
pub mod subscription;
pub mod warp;
pub mod xray;

use anyhow::Result;
use context::{CallbackContext, HandlerAction};

pub async fn dispatch(ctx: &CallbackContext) -> Result<Option<HandlerAction>> {
    let data = ctx.data.as_str();

    if data == "m_log" || data.starts_with("l_") {
        return Ok(Some(log::handle(ctx).await?));
    }

    if data == "m_singbox_mgmt" || data == "sb_install" || data.starts_with("sb_") {
        return Ok(Some(singbox::handle(ctx).await?));
    }

    if data == "m_warp" || data == "a_inst_warp" || data.starts_with("a_warp_") {
        return Ok(Some(warp::handle(ctx).await?));
    }

    if data == "m_sched"
        || data == "a_geo_sched_menu"
        || data == "geo_sched_off"
        || data.starts_with("s_")
    {
        return Ok(Some(schedule::handle(ctx).await?));
    }

    if data.starts_with("a_bbr3")
        || data == "a_fw"
        || data == "a_one_click"
        || data == "a_reload"
        || data == "a_sys_maint"
        || data == "a_sys_reboot"
        || data == "a_upgrade"
        || data == "a_geo"
        || data == "a_tune"
    {
        return Ok(Some(ops::handle(ctx).await?));
    }

    if data == "m_xray_mgmt"
        || data == "m_routing"
        || data.starts_with("routing_toggle:")
        || data == "m_del_cfg"
        || data == "m_pq_mgmt"
        || data == "a_inst_base"
        || data.starts_with("u_")
        || data.starts_with("cfg_")
        || data.starts_with("m_pq_")
    {
        return Ok(Some(xray::handle(ctx).await?));
    }

    if matches!(
        data,
        "m_main"
            | "m_ops_center"
            | "m_settings"
            | "m_net_opt"
            | "m_security"
            | "m_sys_cmd"
            | "m_mon"
            | "m_usr"
            | "m_danger"
            | "m_session_timeout"
            | "a_wwps_core_menu"
            | "a_wwps_box_menu"
            | "a_wwps_box_restart"
            | "a_wwps_box_status"
            | "a_wwps_core_latest"
            | "a_wwps_core_tags"
            | "a_geo_menu"
    ) || data.starts_with("set_timeout:")
        || data.starts_with("wwps_core_tag:")
    {
        return Ok(Some(menu::handle(ctx).await?));
    }

    if data == "m_sub" || data.starts_with("sub_") {
        return Ok(Some(subscription::handle(ctx).await?));
    }

    Ok(None)
}
