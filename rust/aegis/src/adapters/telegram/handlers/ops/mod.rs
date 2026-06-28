use super::context::CallbackContext;
use super::context::{HandlerAction, HandlerResult};
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;

mod bbr3;
mod deploy;
mod firewall;
mod geo;
mod reboot;
mod reload;
mod sys_maint;
mod system;
mod upgrade;

pub async fn handle(ctx: &CallbackContext) -> HandlerResult {
    match ctx.data.as_str() {
        "a_reload" => reload::handle(ctx).await,
        "a_fw" => firewall::handle(ctx).await,
        "a_upgrade" => upgrade::handle(ctx).await,
        "a_geo" => geo::handle(ctx).await,
        "a_tune" | "a_sys_update" => system::handle(ctx).await,
        "a_bbr3"
        | "a_bbr3_confirm"
        | "a_bbr3_cancel"
        | "a_bbr3_reboot_now"
        | "a_bbr3_reboot_later" => bbr3::handle(ctx).await,
        "a_sys_maint" => sys_maint::handle(ctx).await,
        "a_sys_reboot" => reboot::handle(ctx).await,
        "a_one_click" => deploy::handle(ctx).await,
        _ => Ok(HandlerAction::Done),
    }
}

fn spawn_progress_updater(
    bot: Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    title_fn: impl Fn(String) -> String + Send + 'static,
) -> (UnboundedSender<String>, JoinHandle<()>) {
    let (tx, mut rx) = unbounded_channel::<String>();
    let handle = tokio::spawn(async move {
        let mut last = String::new();
        while let Some(text) = rx.recv().await {
            if text == last {
                continue;
            }
            last = text.clone();
            let _ = bot
                .edit_message_text(chat_id, msg_id, title_fn(text))
                .parse_mode(ParseMode::Html)
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    (tx, handle)
}
