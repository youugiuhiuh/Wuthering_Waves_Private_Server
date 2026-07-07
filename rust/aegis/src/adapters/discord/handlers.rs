use std::sync::{Arc, OnceLock};

use serenity::all::{
    Command, CommandInteraction, ComponentInteraction, Context, CreateInteractionResponse,
    CreateInteractionResponseMessage, EventHandler, Interaction, Ready,
};
use serenity::async_trait;
use serenity::http::Http;

use crate::adapters::common::{BotAdapter, MessageId, TargetId};
use crate::app::state::AppState;
use crate::handlers::context::HandlerContext;

use super::commands;

pub struct DiscordHandler {
    pub state: Arc<OnceLock<Arc<AppState>>>,
    pub adapter: Arc<dyn BotAdapter>,
    pub http: Arc<Http>,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, _: Ready) {
        let commands = commands::all_commands();
        if let Err(e) = Command::set_global_commands(&ctx.http, commands).await {
            log::error!("注册 Discord Slash Commands 失败: {}", e);
        } else {
            log::info!("Discord Slash Commands 已注册");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => {
                self.handle_command(ctx, command).await;
            }
            Interaction::Component(component) => {
                self.handle_component(ctx, component).await;
            }
            _ => {}
        }
    }
}

impl DiscordHandler {
    async fn handle_command(&self, _ctx: Context, command: CommandInteraction) {
        let Some(state) = self.state.get() else {
            log::error!("Discord handler state not initialized");
            return;
        };

        let user_id = command.user.id.get() as i64;

        let command_name = command.data.name.as_str();
        if command_name != "auth" && !state.is_authorized(user_id).await {
            let _ = command
                .create_response(
                    &self.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("⛔ 请先使用 /auth 验证 TOTP 验证码")
                            .ephemeral(true),
                    ),
                )
                .await;
            return;
        }

        let data = build_command_data(&command);
        let target = TargetId(command.channel_id.to_string());

        // Defer response to avoid 3s timeout
        let _ = command
            .create_response(
                &self.http,
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().ephemeral(false),
                ),
            )
            .await;

        // Get the deferred message ID for edit tracking
        let msg_id = command
            .get_response(&self.http)
            .await
            .ok()
            .map(|msg| MessageId(msg.id.to_string()));

        let hctx = HandlerContext {
            adapter: &*self.adapter,
            target,
            state,
            user_id,
            data,
            msg_id,
        };

        match crate::handlers::dispatch::dispatch(&hctx).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Discord command handler error: {:?}", e);
            }
        }
    }

    async fn handle_component(&self, _ctx: Context, component: ComponentInteraction) {
        let Some(state) = self.state.get() else {
            log::error!("Discord handler state not initialized");
            return;
        };

        let user_id = component.user.id.get() as i64;

        if !state.is_authorized(user_id).await {
            let _ = component
                .create_response(
                    &self.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("⛔ 验证已过期，请重新验证")
                            .ephemeral(true),
                    ),
                )
                .await;
            return;
        }

        let hctx = HandlerContext {
            adapter: &*self.adapter,
            target: TargetId(component.channel_id.to_string()),
            state,
            user_id,
            data: component.data.custom_id.clone(),
            msg_id: Some(MessageId(component.message.id.to_string())),
        };

        let _ = component
            .create_response(&self.http, CreateInteractionResponse::Acknowledge)
            .await;

        match crate::handlers::dispatch::dispatch(&hctx).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Discord component handler error: {:?}", e);
            }
        }
    }
}

fn build_command_data(command: &CommandInteraction) -> String {
    let name = command.data.name.as_str();

    if let Some(option) = command.data.options.first() {
        match &option.value {
            serenity::all::CommandDataOptionValue::SubCommandGroup(sub_options) => {
                if let Some(sub) = sub_options.first() {
                    format!("a_{}_{}", name, sub.name)
                } else {
                    format!("a_{}", name)
                }
            }
            serenity::all::CommandDataOptionValue::SubCommand(_) => {
                format!("a_{}_{}", name, option.name)
            }
            _ => format!("a_{}", name),
        }
    } else {
        match name {
            "menu" => "m_main".to_string(),
            "status" => "m_status".to_string(),
            _ => format!("a_{}", name),
        }
    }
}
