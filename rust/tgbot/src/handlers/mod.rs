pub mod context;
pub mod log;
pub mod menu;
pub mod ops;
pub mod schedule;
pub mod singbox;
pub mod warp;
pub mod xray;
pub(crate) mod callback;
pub(crate) mod message;

use context::{CallbackContext, HandlerAction};
use anyhow::Result;

pub async fn dispatch(ctx: &CallbackContext) -> Result<Option<HandlerAction>> {
    let data = ctx.data.as_str();

    if data == "m_log" || data == "l_xray" || data == "l_box" || data == "l_xray_tail" || data == "l_box_tail" {
        return Ok(Some(log::handle(ctx).await?));
    }

    let singbox_exact = [
        "m_singbox_mgmt",
        "sb_install",
        "sb_h2_init",
        "sb_tu_init",
        "sb_del_cfg",
        "sb_del_all_confirm",
        "sb_del_all_exec",
        "sb_del_count",
        "sb_del_select",
    ];
    if singbox_exact.contains(&data) {
        return Ok(Some(singbox::handle(ctx).await?));
    }

    let singbox_prefixes = [
        "sb_h2_ip:",
        "sb_h2_obfs:",
        "sb_h2_exec:",
        "sb_tu_ip:",
        "sb_tu_exec:",
        "sb_del_exec_count:",
        "sb_del_file:",
        "sb_l:",
    ];
    for prefix in &singbox_prefixes {
        if data.starts_with(prefix) {
            return Ok(Some(singbox::handle(ctx).await?));
        }
    }

    if data == "m_sched" || data == "s_add_menu" || data == "s_add_custom_menu"
        || data == "s_custom_confirm" || data == "s_custom_cancel" || data == "s_del_menu"
        || data == "a_geo_sched_menu" || data == "geo_sched_off"
        || data.starts_with("s_custom") || data.starts_with("s_add:")
        || data.starts_with("s_del:") || data.starts_with("s_del_confirm:")
    {
        return Ok(Some(schedule::handle(ctx).await?));
    }

    if data == "m_warp" || data.starts_with("a_warp_") {
        return Ok(Some(warp::handle(ctx).await?));
    }

    if data.starts_with("a_bbr3") || data == "a_fw" || data == "a_reload"
        || data == "a_sys_maint" || data == "a_sys_reboot" || data == "a_tune"
        || data == "a_upgrade" || data == "a_geo"
    {
        return Ok(Some(ops::handle(ctx).await?));
    }

    if data == "m_xray_mgmt" || data == "m_del_cfg" || data == "m_pq_mgmt" || data == "a_inst_base"
        || data.starts_with("u_") || data.starts_with("cfg_") || data.starts_with("m_pq_")
    {
        return Ok(Some(xray::handle(ctx).await?));
    }

    let menu_patterns = [
        "m_main", "m_ops_center", "m_settings", "m_net_opt", "m_security",
        "m_sys_cmd", "m_mon", "m_usr", "m_danger", "m_session_timeout",
        "a_wwps_core_menu", "a_wwps_core_latest", "a_wwps_core_tags",
        "a_wwps_box_menu", "a_wwps_box_restart", "a_wwps_box_status",
        "a_geo_menu",
    ];
    if menu_patterns.contains(&data) || data.starts_with("set_timeout:") || data.starts_with("wwps_core_tag:") {
        return Ok(Some(menu::handle(ctx).await?));
    }

    Ok(None)
}