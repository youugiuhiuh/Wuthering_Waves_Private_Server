use super::context::{HandlerAction, HandlerContext, HandlerResult};

pub async fn dispatch(ctx: &HandlerContext<'_>) -> HandlerResult {
    let data = ctx.data.as_str();

    // 菜单系统
    if matches!(
        data,
        "m_main" | "m_ops_center" | "m_settings" | "m_sub" | "m_sched"
    ) || data.starts_with("m_")
    {
        return super::menu::handle(ctx).await;
    }

    // WARP (must check before a_* catch-all)
    if data.starts_with("a_warp_") {
        return super::warp::handle(ctx).await;
    }

    // 运维操作
    if data.starts_with("a_") {
        return super::ops::handle(ctx).await;
    }

    // Xray 管理
    if data.starts_with("u_") || data.starts_with("cfg_") || data.starts_with("routing_") {
        return super::xray::handle(ctx).await;
    }

    // SingBox
    if data.starts_with("sb_") {
        return super::singbox::handle(ctx).await;
    }

    // 订阅
    if data.starts_with("sub_") {
        return super::subscription::handle(ctx).await;
    }

    // 调度
    if data.starts_with("s_") {
        return super::schedule::handle(ctx).await;
    }

    // 日志
    if data.starts_with("l_") {
        return super::log::handle(ctx).await;
    }

    Ok(HandlerAction::Done)
}
