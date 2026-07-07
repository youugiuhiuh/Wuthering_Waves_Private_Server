use std::sync::Arc;

use aegis::adapters::common::{BotAdapter, RoutingAdapter};
use aegis::adapters::telegram::TelegramAdapter;
use anyhow::Result;
use teloxide::Bot;

use crate::register_bot_commands;

pub async fn build_adapter(
    token: &str,
    enable_telegram: bool,
    enable_matrix: bool,
    enable_discord: bool,
    matrix_handle: &Option<super::matrix::MatrixHandle>,
    discord_handle: &Option<super::discord::DiscordHandle>,
) -> Result<Arc<dyn BotAdapter>> {
    if enable_telegram {
        let tg_adapter = {
            let bot = Bot::new(token);
            if let Err(err) = register_bot_commands(&bot).await {
                eprintln!("[WARN] 命令注册失败: {}", err);
            }
            Arc::new(TelegramAdapter::new(bot)) as Arc<dyn BotAdapter>
        };
        let secondary = if enable_matrix {
            matrix_handle.as_ref().map(|(_, _, a)| a.clone())
        } else if enable_discord {
            discord_handle.as_ref().map(|h| h.adapter.clone())
        } else {
            None
        };
        Ok(Arc::new(RoutingAdapter::new(tg_adapter, secondary)))
    } else if enable_discord {
        Ok(discord_handle.as_ref().unwrap().adapter.clone())
    } else if let Some((_, _, matrix_adapter)) = matrix_handle {
        Ok(matrix_adapter.clone())
    } else {
        anyhow::bail!("没有启用任何平台");
    }
}
