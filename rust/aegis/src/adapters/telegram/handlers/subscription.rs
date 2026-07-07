use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::app::state::{AppState, SubSetupStep};
use aegis::handlers::context::HandlerContext;
use aegis::adapters::common::{MessageId, TargetId};
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let hctx = HandlerContext {
        adapter: &*ctx.state.adapter,
        target: TargetId(ctx.chat_id.0.to_string()),
        state: &ctx.state,
        user_id: ctx.user_id,
        data: ctx.data.clone(),
        msg_id: Some(MessageId(ctx.msg_id.0.to_string())),
    };
    aegis::handlers::subscription::handle(&hctx).await.map(HandlerAction::from)
}

pub async fn handle_text_input(
    bot: &Bot,
    chat_id: ChatId,
    state: &AppState,
    text: &str,
) -> Result<bool, teloxide::RequestError> {
    let chat_id_str = chat_id.0.to_string();
    let Some(mut setup) = state.sub_setup_status(&chat_id_str).await else {
        return Ok(false);
    };

    match setup.step {
        SubSetupStep::EnterDomain => {
            if text.is_empty() || text.contains(' ') || !text.contains('.') {
                bot.send_message(chat_id, t!("sub.setup_q_domain_input"))
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(true);
            }
            setup.domain = text.trim().to_string();
            setup.step = SubSetupStep::EnterPort;
            state.insert_sub_setup(chat_id_str, setup).await;
            bot.send_message(chat_id, t!("sub.setup_q_port"))
                .parse_mode(ParseMode::Html)
                .await?;
        }
        SubSetupStep::EnterPort => {
            let port: u16 = match text.trim().parse() {
                Ok(p) if (1024..=65535).contains(&p) => p,
                _ => 8443,
            };
            setup.port = port;
            setup.step = SubSetupStep::EnterRateLimit;
            state.insert_sub_setup(chat_id_str, setup).await;
            bot.send_message(chat_id, t!("sub.setup_q_rate"))
                .parse_mode(ParseMode::Html)
                .await?;
        }
        SubSetupStep::EnterRateLimit => {
            let rate: u32 = match text.trim().parse() {
                Ok(r) if (1..=100).contains(&r) => r,
                _ => 10,
            };
            setup.rate_limit = rate;
            setup.step = SubSetupStep::ChooseTls;
            state.insert_sub_setup(chat_id_str, setup).await;
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_le"),
                    "sub_tls:le",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_ip"),
                    "sub_tls:ip",
                )],
                vec![InlineKeyboardButton::callback(
                    t!("sub.setup_cert_self"),
                    "sub_tls:self",
                )],
                vec![InlineKeyboardButton::callback(t!("menu.back"), "m_sub")],
            ]);
            bot.send_message(chat_id, t!("sub.setup_q_cert"))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        SubSetupStep::ChooseDomain | SubSetupStep::ChooseTls | SubSetupStep::Confirm => {}
    }
    Ok(true)
}
