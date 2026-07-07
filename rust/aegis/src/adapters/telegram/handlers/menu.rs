use super::context::{CallbackContext, HandlerAction, HandlerResult};
use aegis::handlers::context::HandlerContext;
use aegis::adapters::common::{MessageId, TargetId};
use rust_i18n::t;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn send_main_menu(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let mut rows = vec![
        vec![
            InlineKeyboardButton::callback(t!("menu.monitor"), "m_mon"),
            InlineKeyboardButton::callback(t!("menu.users"), "m_usr"),
        ],
        vec![InlineKeyboardButton::callback(
            t!("menu.ops"),
            "m_ops_center",
        )],
        vec![InlineKeyboardButton::callback(
            t!("menu.settings"),
            "m_settings",
        )],
    ];
    rows.push(vec![InlineKeyboardButton::callback(
        t!("menu.one_click_deploy"),
        "a_one_click",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        t!("menu.sub_service"),
        "m_sub",
    )]);
    if !aegis::core::i18n::is_lang_configured() {
        rows.push(vec![
            InlineKeyboardButton::callback(t!("lang.zh"), "lang:zh"),
            InlineKeyboardButton::callback(t!("lang.en"), "lang:en"),
            InlineKeyboardButton::callback(t!("lang.ja"), "lang:ja"),
        ]);
    }
    let keyboard = InlineKeyboardMarkup::new(rows);
    bot.send_message(
        chat_id,
        format!("{}\n{}", t!("menu.title"), t!("menu.prompt")),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    let hctx = HandlerContext {
        adapter: &*ctx.state.adapter,
        target: TargetId(ctx.chat_id.0.to_string()),
        state: &ctx.state,
        user_id: ctx.user_id,
        data: ctx.data.clone(),
        msg_id: Some(MessageId(ctx.msg_id.0.to_string())),
    };
    aegis::handlers::menu::handle(&hctx).await.map(HandlerAction::from)
}
