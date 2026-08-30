pub(crate) mod callback;
pub mod log;
pub mod menu;
pub mod message;
pub mod ops;
pub mod schedule;
pub mod singbox;
pub mod warp;
pub mod xray;

use crate::app::state::AppState;
use crate::shared::types::{CallbackEvent, DispatchResult};

/// Handler a callback data string is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallbackRoute {
    Log,
    Singbox,
    Warp,
    Schedule,
    Ops,
    Xray,
    Menu,
}

/// Routes an inline-button callback `data` string to its handler.
///
/// # Contract (防止“加按钮忘注册回调”)
///
/// 任何菜单渲染出的按钮回调 ID 必须在此解析为 `Some(_)`，或由 `shared/dispatch.rs`
/// 中更早的拦截层处理（`lang:` / `set_timeout:` / `a_warp_add_input` 由
/// `state_ops::intercept` 拦截，`a_destroy_*` 由 `destruct::intercept_callback` 拦截）。
/// 否则回调会被静默丢弃（按钮点击无任何响应）。
///
/// 1.2.7（commit `adb28ef`）曾因漏注册 `a_wwps_core_restart` / `a_wwps_core_status`
/// 导致“重启 wwps-core / 状态”按钮完全失效。回归测试
/// [`test_every_menu_button_data_is_routed`](self::tests::test_every_menu_button_data_is_routed)
/// 扫描 `menu.rs` 的所有按钮 data 字面量，新增按钮未注册时会直接失败。
///
/// # 已知限制
///
/// 服务重启/状态检查（`WwpsCoreUpgradeManager::restart_service`、
/// `SystemMonitor::check_service_status`）目前假定 systemd；安装器
/// （`xray/installer.rs::install_wwps_core_service`）同时支持 OpenRC，若部署到
/// OpenRC 主机需同步扩展这两处。
pub(crate) fn route_callback(data: &str) -> Option<CallbackRoute> {
    if data == "m_log" || data.starts_with("l_") {
        return Some(CallbackRoute::Log);
    }
    if data == "m_singbox_mgmt" || data == "sb_install" || data.starts_with("sb_") {
        return Some(CallbackRoute::Singbox);
    }
    if data == "m_warp" || data == "a_inst_warp" || data.starts_with("a_warp_") {
        return Some(CallbackRoute::Warp);
    }
    if data == "m_sched"
        || data == "a_geo_sched_menu"
        || data == "geo_sched_off"
        || data == "s_add_menu"
        || data == "s_del_menu"
        || data.starts_with("s_add:")
        || data.starts_with("s_del:")
    {
        return Some(CallbackRoute::Schedule);
    }
    if data.starts_with("a_bbr3")
        || data == "a_fw"
        || data == "a_one_click"
        || data == "a_reload"
        || data == "a_sys_reboot"
        || data == "a_upgrade"
        || data == "a_geo"
        || data == "a_tune"
        || data == "a_sys_update"
    {
        return Some(CallbackRoute::Ops);
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
        || data.starts_with("xhttp_domain_")
    {
        return Some(CallbackRoute::Xray);
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
            | "a_wwps_core_restart"
            | "a_wwps_core_status"
            | "a_geo_menu"
    ) || data.starts_with("set_timeout:")
        || data.starts_with("wwps_core_tag:")
    {
        return Some(CallbackRoute::Menu);
    }
    None
}

pub async fn dispatch(event: &CallbackEvent, state: &AppState) -> DispatchResult {
    let data = event.data.as_str();

    match route_callback(data) {
        Some(CallbackRoute::Log) => Ok(Some(self::log::handle(event).await?)),
        Some(CallbackRoute::Singbox) => Ok(Some(singbox::handle(event).await?)),
        Some(CallbackRoute::Warp) => Ok(Some(warp::handle(event).await?)),
        Some(CallbackRoute::Schedule) => Ok(Some(schedule::handle(event).await?)),
        Some(CallbackRoute::Ops) => Ok(Some(ops::handle(event).await?)),
        Some(CallbackRoute::Xray) => Ok(Some(xray::handle(event, state).await?)),
        Some(CallbackRoute::Menu) => Ok(Some(menu::handle(event).await?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Callback ids rendered by menu.rs but handled *before* `dispatch` reaches
    /// `route_callback` — see `shared/dispatch.rs` pipeline order:
    /// `destruct::intercept_callback` (`a_destroy_*`) runs first, then
    /// `state_ops::intercept` (`lang:`, `set_timeout:`, `a_warp_add_input`).
    fn handled_before_dispatch(data: &str) -> bool {
        data.starts_with("lang:")
            || data.starts_with("set_timeout:")
            || data.starts_with("a_destroy_")
            || data == "a_warp_add_input"
    }

    /// Zero-dependency extractor for `data: "..."` literals (inline button ids)
    /// from the menu.rs source text.
    fn extract_data_literals(source: &str) -> Vec<&str> {
        let mut ids = Vec::new();
        let mut rest = source;
        while let Some(pos) = rest.find("data:") {
            rest = &rest[pos + "data:".len()..];
            let after = rest.trim_start();
            if let Some(after_quote) = after.strip_prefix('"') {
                if let Some(end) = after_quote.find('"') {
                    ids.push(&after_quote[..end]);
                    rest = &after_quote[end + 1..];
                } else {
                    break;
                }
            }
        }
        ids
    }

    /// Regression for 1.2.7 (commit adb28ef): the two wwps-core buttons were
    /// added to menu.rs but never registered in the dispatch whitelist, so
    /// clicks were silently dropped (`Ok(None)`) and the message never updated.
    #[test]
    fn test_wwps_core_restart_and_status_route_to_menu() {
        assert_eq!(
            route_callback("a_wwps_core_restart"),
            Some(CallbackRoute::Menu)
        );
        assert_eq!(
            route_callback("a_wwps_core_status"),
            Some(CallbackRoute::Menu)
        );
    }

    /// Every inline-button id rendered by menu.rs must be routable: either by
    /// `route_callback` or by an earlier interceptor. Prevents the
    /// “button added but callback never registered” class of bug.
    #[test]
    fn test_every_menu_button_data_is_routed() {
        let source = include_str!("menu.rs");
        let unregistered: Vec<&str> = extract_data_literals(source)
            .into_iter()
            .filter(|id| route_callback(id).is_none() && !handled_before_dispatch(id))
            .collect();
        assert!(
            unregistered.is_empty(),
            "menu.rs 中以下按钮回调 ID 未注册路由（请在 route_callback 或前置拦截层处理）: {:?}",
            unregistered
        );
    }

    #[test]
    fn test_unknown_callback_is_none() {
        assert_eq!(route_callback("no_such_callback_xyz"), None);
        assert_eq!(route_callback(""), None);
    }
}
