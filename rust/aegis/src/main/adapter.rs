use std::sync::Arc;

use aegis::common::{BotAdapter, RoutingAdapter};
use aegis::gateways::telegram::TelegramAdapter;
use anyhow::{Context, Result};
use teloxide::Bot;

use crate::main::runtime::register_bot_commands;

pub async fn build_adapter(
    token: Option<&str>,
    enable_telegram: bool,
    enable_matrix: bool,
    matrix_handle: &Option<super::matrix::MatrixHandle>,
) -> Result<Arc<dyn BotAdapter>> {
    if enable_telegram {
        let token = token.context("Telegram token is required when enable_telegram is true")?;
        if enable_matrix {
            let tg_adapter = {
                let bot = Bot::new(token);
                if let Err(err) = register_bot_commands(&bot).await {
                    eprintln!("[WARN] 命令注册失败: {}", err);
                }
                Arc::new(TelegramAdapter::new(bot)) as Arc<dyn BotAdapter>
            };
            let secondary = matrix_handle.as_ref().map(|(_, _, a)| a.clone());
            Ok(Arc::new(RoutingAdapter::new(tg_adapter, secondary)))
        } else {
            let bot = Bot::new(token);
            if let Err(err) = register_bot_commands(&bot).await {
                eprintln!("[WARN] 命令注册失败: {}", err);
            }
            Ok(Arc::new(TelegramAdapter::new(bot)))
        }
    } else if let Some((_, _, matrix_adapter)) = matrix_handle {
        Ok(matrix_adapter.clone())
    } else {
        anyhow::bail!("没有启用任何平台，请使用 --matrix 或 --all 或省略参数使用 Telegram");
    }
}
