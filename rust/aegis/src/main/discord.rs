use std::sync::{Arc, OnceLock};

use aegis::adapters::common::BotAdapter;
use aegis::adapters::discord::DiscordAdapter;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use serenity::all::GatewayIntents;
use serenity::client::Client;
use serenity::http::Http;

use crate::bootstrap::EncryptedConfig;
use aegis::adapters::discord::handlers::DiscordHandler;
use aegis::app::state::AppState;
use aegis::core::security::SecurityManager;

pub struct DiscordHandle {
    pub client: Client,
    pub adapter: Arc<dyn BotAdapter>,
    pub http: Arc<Http>,
    pub state: Arc<OnceLock<Arc<AppState>>>,
}

pub fn has_discord_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit_discord = args.iter().any(|a| *a == "--discord" || *a == "--all");
    explicit_discord || encrypted_config.discord_token.is_some()
}

pub async fn connect_discord(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
) -> Result<DiscordHandle> {
    let token_raw = if let Some(ref token) = encrypted_config.discord_token {
        let decrypted = security.decrypt(token)?;
        String::from_utf8(decrypted.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("Discord token 包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string()
    } else {
        std::env::var("DISCORD_TOKEN")
            .context("未设置 DISCORD_TOKEN 环境变量且 config.enc 中没有 discord_token")?
    };

    let http = Arc::new(Http::new(&token_raw));
    let adapter: Arc<dyn BotAdapter> = Arc::new(DiscordAdapter::new(http.clone()));
    let state_cell: Arc<OnceLock<Arc<AppState>>> = Arc::new(OnceLock::new());

    let handler = DiscordHandler {
        state: state_cell.clone(),
        adapter: adapter.clone(),
        http: http.clone(),
    };

    let intents = GatewayIntents::non_privileged();
    let client = Client::builder(&token_raw, intents)
        .event_handler(handler)
        .await
        .context("创建 Discord 客户端失败")?;

    Ok(DiscordHandle {
        client,
        adapter,
        http,
        state: state_cell,
    })
}
