use super::super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::core::xray::routing::RoutingManager;
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub(super) async fn handle_routing_menu(ctx: &CallbackContext) -> HandlerResult {
    let rules = RoutingManager::get_all_with_status()
        .await
        .map_err(|e| anyhow::anyhow!("获取路由规则失败: {}", e))?;

    let active_count = rules.iter().filter(|(_, enabled)| *enabled).count();
    let mut text = t!("xray.routing_title").to_string();
    text.push_str(&format!(
        "\n\n{}",
        t!("xray.routing_active_count", "count" => active_count.to_string())
    ));

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = rules
        .iter()
        .map(|(def, enabled)| {
            let i18n_key = format!("xray.routing_rule_{}", def.id);
            let name = t!(i18n_key.as_str());
            let icon = if *enabled { "✅" } else { "⬜" };
            vec![InlineKeyboardButton::callback(
                format!("{} {}", icon, name),
                format!("routing_toggle:{}", def.id),
            )]
        })
        .collect();

    buttons.push(vec![InlineKeyboardButton::callback(
        t!("menu.back"),
        "m_xray_mgmt",
    )]);

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(buttons))
        .await?;

    Ok(HandlerAction::Done)
}

pub(super) async fn handle_routing_toggle(ctx: &CallbackContext, data: &str) -> HandlerResult {
    let rule_id = data.strip_prefix("routing_toggle:").unwrap_or("");
    if rule_id.is_empty() {
        return Ok(HandlerAction::Redirect("m_routing".to_string()));
    }

    match RoutingManager::toggle(rule_id).await {
        Ok(enabled) => {
            let i18n_key = format!("xray.routing_rule_{}", rule_id);
            let name = t!(i18n_key.as_str());
            let msg = if enabled {
                t!("xray.routing_toggled_on", "name" => name)
            } else {
                t!("xray.routing_toggled_off", "name" => name)
            };
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(msg)
                .await?;
        }
        Err(e) => {
            ctx.bot
                .answer_callback_query(ctx.q.id.clone())
                .text(format!("{}: {}", t!("xray.routing_reload_failed"), e))
                .await?;
        }
    }

    Ok(HandlerAction::Redirect("m_routing".to_string()))
}
