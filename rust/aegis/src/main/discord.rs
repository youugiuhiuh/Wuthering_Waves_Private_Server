use std::path::Path;
use std::sync::Arc;

use aegis::app::state::AppState;
use aegis::common::BotAdapter;
use aegis::gateways::discord::DiscordAdapter;
use aegis::shared::dispatch_event;
use aegis::shared::types::*;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use serenity::all::{
    ChannelId, Command, CommandDataOptionValue, CreateCommand, GatewayIntents, Interaction,
    Message, UserId,
};
use serenity::async_trait;
use serenity::client::{Client, Context as SerenityCtx, EventHandler};
use serenity::http::Http;

use crate::bootstrap::EncryptedConfig;

/// Raw materials for Discord runtime wiring, built before AppState exists.
#[allow(dead_code)]
pub struct DiscordRawHandle {
    pub token: String,
    pub http: Arc<Http>,
    pub admin_channel: ChannelId,
    pub adapter: Arc<dyn BotAdapter>,
    pub admin_id: u64,
}

/// Discord runtime handle: Client (gateway), ChannelId, Adapter.
#[allow(dead_code)]
pub type DiscordHandle = (Client, ChannelId, Arc<dyn BotAdapter>);

#[allow(dead_code)]
fn parse_slash(name: &str, code: Option<&str>) -> Option<BotCommand> {
    match name {
        "help" => Some(BotCommand::Help),
        "start" => Some(BotCommand::Start),
        "menu" => Some(BotCommand::Menu),
        "auth" => code.map(|c| BotCommand::Auth {
            code: c.to_string(),
        }),
        "setsecurityfile" => Some(BotCommand::SetSecurityFile),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn has_discord_config(enc: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--discord" || a == "--all");
    explicit || (enc.discord_token.is_some() && enc.discord_admin_id.is_some())
}

#[allow(dead_code)]
pub async fn register_slash_commands(http: &Http) -> Result<()> {
    use serenity::all::CreateCommandOption;
    Command::set_global_commands(
        http,
        vec![
            CreateCommand::new("help").description("Show help"),
            CreateCommand::new("start").description("Start bot"),
            CreateCommand::new("menu").description("Show admin menu"),
            CreateCommand::new("auth")
                .description("Verify TOTP code")
                .add_option(
                    CreateCommandOption::new(
                        serenity::all::CommandOptionType::String,
                        "code",
                        "6-digit TOTP code",
                    )
                    .required(true),
                ),
            CreateCommand::new("setsecurityfile").description("Set destruct verification file"),
        ],
    )
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn connect_discord(
    security: &aegis::core::security::SecurityManager,
    enc: &EncryptedConfig,
    _config_dir: &Path,
) -> Result<DiscordRawHandle> {
    let decrypt = |field: &Option<Vec<u8>>| -> Result<String> {
        let vec = security.decrypt(field.as_ref().with_context(|| "缺少 Discord 配置项")?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("Discord 字段包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string())
    };

    let discord_token = decrypt(&enc.discord_token)?;
    let discord_admin_id_str = decrypt(&enc.discord_admin_id)?;
    let discord_admin_id: u64 = discord_admin_id_str
        .parse()
        .context("discord_admin_id 应为用户 ID(数字)")?;

    let http = Arc::new(Http::new(&discord_token));
    let admin_user = UserId::new(discord_admin_id);
    let admin_channel = admin_user.create_dm_channel(&http).await?.id;
    let adapter: Arc<dyn BotAdapter> = Arc::new(DiscordAdapter::new(http.clone()));

    register_slash_commands(&http).await?;

    Ok(DiscordRawHandle {
        token: discord_token,
        http,
        admin_channel,
        adapter,
        admin_id: discord_admin_id,
    })
}

/// Build a (Client, ChannelId, Adapter) tuple from a RawHandle + AppState.
/// Called in runtime.rs where AppState exists.
#[allow(dead_code)]
pub async fn build_handle(raw: DiscordRawHandle, state: Arc<AppState>) -> Result<DiscordHandle> {
    let intents = GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let handler = DiscordHandler {
        state,
        adapter: raw.adapter.clone(),
        admin_channel: raw.admin_channel,
    };
    let client = Client::builder(&raw.token, intents)
        .event_handler(handler)
        .await
        .context("构建 Discord Client 失败")?;
    Ok((client, raw.admin_channel, raw.adapter))
}

#[allow(dead_code)]
struct DiscordHandler {
    state: Arc<AppState>,
    adapter: Arc<dyn BotAdapter>,
    admin_channel: ChannelId,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, _ctx: SerenityCtx, msg: Message) {
        if msg.channel_id != self.admin_channel {
            return;
        }
        let user_id = msg.author.id.get() as i64;
        if !self.state.is_admin_user(user_id) {
            return;
        }
        let text = Some(msg.content).filter(|s| !s.is_empty());
        let (file_id, file_name) = match msg.attachments.first() {
            Some(a) => (Some(a.url.to_string()), Some(a.filename.clone())),
            None => (None, None),
        };
        let event = BotEvent::Message(MessageEvent {
            adapter: self.adapter.clone(),
            target: aegis::common::TargetId(self.admin_channel.to_string()),
            user_id,
            text,
            file_id,
            file_name,
            reply_to_text: None,
        });
        let _ = dispatch_event(event, &self.state).await;
    }

    async fn interaction_create(&self, ctx: SerenityCtx, interaction: Interaction) {
        let _ = match interaction {
            Interaction::Command(ref cmd) => {
                let _ = cmd.defer(&ctx.http).await;
                let name = cmd.data.name.as_str();
                let code = cmd.data.options.first().and_then(|opt| match &opt.value {
                    CommandDataOptionValue::String(s) => Some(s.as_str()),
                    _ => None,
                });
                if let Some(command) = parse_slash(name, code) {
                    let event = BotEvent::Command(CommandEvent {
                        adapter: self.adapter.clone(),
                        target: aegis::common::TargetId(cmd.channel_id.to_string()),
                        user_id: cmd.user.id.get() as i64,
                        command,
                    });
                    dispatch_event(event, &self.state).await
                } else {
                    Ok(())
                }
            }
            Interaction::Component(ref comp) => {
                let _ = comp.defer(&ctx.http).await;
                let msg = &comp.message;
                let event = BotEvent::Callback(CallbackEvent {
                    adapter: self.adapter.clone(),
                    target: aegis::common::TargetId(msg.channel_id.to_string()),
                    user_id: comp.user.id.get().to_string(),
                    msg_id: aegis::common::MessageId(msg.id.to_string()),
                    data: comp.data.custom_id.clone(),
                    callback_id: String::new(),
                    session_timeout_secs: self.state.session_timeout_secs().await,
                });
                dispatch_event(event, &self.state).await
            }
            _ => Ok(()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::shared::types::BotCommand;

    fn make_enc() -> EncryptedConfig {
        EncryptedConfig {
            token: vec![],
            admin_id: vec![],
            totp_secret: vec![],
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
        }
    }

    #[test]
    fn parse_slash_known_commands() {
        assert_eq!(parse_slash("help", None), Some(BotCommand::Help));
        assert_eq!(parse_slash("start", None), Some(BotCommand::Start));
        assert_eq!(parse_slash("menu", None), Some(BotCommand::Menu));
        assert_eq!(
            parse_slash("auth", Some("123456")),
            Some(BotCommand::Auth {
                code: "123456".into()
            })
        );
        assert_eq!(
            parse_slash("setsecurityfile", None),
            Some(BotCommand::SetSecurityFile)
        );
    }

    #[test]
    fn parse_slash_unknown_returns_none() {
        assert_eq!(parse_slash("unknown", None), None);
        assert_eq!(parse_slash("auth", None), None);
    }

    #[test]
    fn has_discord_config_by_flag() {
        let enc = make_enc();
        assert!(has_discord_config(&enc, &["--discord".to_string()]));
        assert!(has_discord_config(&enc, &["--all".to_string()]));
    }

    #[test]
    fn has_discord_config_by_fields() {
        let mut enc = make_enc();
        enc.discord_token = Some(vec![1]);
        enc.discord_admin_id = Some(vec![2]);
        assert!(has_discord_config(&enc, &[]));
    }

    #[test]
    fn has_discord_config_false_when_missing() {
        let enc = make_enc();
        assert!(!has_discord_config(&enc, &[]));
    }
}
