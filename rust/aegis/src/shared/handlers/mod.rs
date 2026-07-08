pub(crate) mod callback;
// Additional handler modules will be added by Tasks 4–11:
// pub(crate) mod log;
// pub(crate) mod menu;
// pub(crate) mod message;
// pub(crate) mod ops;
// pub(crate) mod schedule;
// pub(crate) mod singbox;
// pub(crate) mod warp;
// pub(crate) mod xray;

use crate::shared::types::{CallbackEvent, DispatchResult, HandlerAction};

pub async fn dispatch(event: &CallbackEvent) -> DispatchResult {
    let data = event.data.as_str();

    if data == "m_log" || data.starts_with("l_") {
        return Ok(Some(HandlerAction::Done));
    }
    if data == "m_singbox_mgmt" || data == "sb_install" || data.starts_with("sb_") {
        return Ok(Some(HandlerAction::Done));
    }
    if data == "m_warp" || data == "a_inst_warp" || data.starts_with("a_warp_") {
        return Ok(Some(HandlerAction::Done));
    }
    if data == "m_sched"
        || data == "a_geo_sched_menu"
        || data == "geo_sched_off"
        || data.starts_with("s_")
    {
        return Ok(Some(HandlerAction::Done));
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
        return Ok(Some(HandlerAction::Done));
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
        // Task 9 will replace with xray::handle(event).await
        return Ok(Some(HandlerAction::Done));
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
        return Ok(Some(HandlerAction::Done));
    }

    Ok(None)
}
