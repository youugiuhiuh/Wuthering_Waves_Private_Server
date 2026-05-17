pub mod context;
pub mod log;
pub mod singbox;
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

    Ok(None)
}