pub mod context;
pub mod log; // 引入日志模块

use context::{CallbackContext, HandlerAction};
use anyhow::Result;

pub async fn dispatch(ctx: &CallbackContext) -> Result<Option<HandlerAction>> {
    let data = ctx.data.as_str();

    // 🔬 安检规则：如果是日志相关的按钮，直接分流给 log.rs
    if data == "m_log" || data == "l_xray" || data == "l_box" || data == "l_xray_tail" || data == "l_box_tail" {
        return Ok(Some(log::handle(ctx).await?));
    }

    // 如果不是日志，返回 None，放行给 main.rs 的老 match 块
    Ok(None)
}