use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;

use crate::adapters::common::{BotAdapter, MessageContent, TargetId};
use crate::core::subscription::config::SubscriptionConfig;
use crate::core::xray::config::ConfigManager;
use crate::shared::types::TimeoutStatus;

const MAX_INPUT_LENGTH: usize = 4096;

pub enum MessageAction {
    Handled,
    NeedsDestruct,
}

#[async_trait]
pub trait MessageState: Send + Sync {
    async fn schedule_timeout_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn remove_schedule_input(&self, chat_id: &str);
    async fn take_warp_input_status(&self, chat_id: &str, timeout: Duration) -> TimeoutStatus;
    async fn subscription_input_timeout_status(
        &self,
        chat_id: &str,
        timeout: Duration,
    ) -> TimeoutStatus;
    async fn cancel_subscription_input(&self, chat_id: &str);
    async fn take_subscription_input(
        &self,
        chat_id: &str,
    ) -> Option<(
        crate::shared::handlers::subscription::SubscriptionInput,
        SubscriptionConfig,
    )>;
}

pub async fn handle_message(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    text: Option<&str>,
    has_file: bool,
    state: &dyn MessageState,
) -> anyhow::Result<MessageAction> {
    // Input length check
    if let Some(t) = text
        && t.len() > MAX_INPUT_LENGTH
    {
        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("message.input_too_long", "0" => MAX_INPUT_LENGTH.to_string())
                        .to_string(),
                    markup: None,
                },
            )
            .await?;
        return Ok(MessageAction::Handled);
    }

    let target_str = &target.0;

    // Subscription typed input intercept — must run before schedule/WARP
    match state
        .subscription_input_timeout_status(target_str, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            state.cancel_subscription_input(target_str).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("subscription.input_timeout").to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::Active => {
            if text.is_some() || has_file {
                return Ok(MessageAction::Handled);
            }
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::NotTracked => {}
    }

    // Schedule timeout check
    match state
        .schedule_timeout_status(target_str, Duration::from_secs(180))
        .await
    {
        TimeoutStatus::Expired => {
            state.remove_schedule_input(target_str).await;
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("schedule.input_timeout").to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::Active => {
            if text.is_some() || has_file {
                adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: t!("schedule.input_prompt").to_string(),
                            markup: None,
                        },
                    )
                    .await?;
            }
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::NotTracked => {}
    }

    // Warp input check
    match state
        .take_warp_input_status(target_str, Duration::from_secs(60))
        .await
    {
        TimeoutStatus::Expired => {
            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("message.warp_input_timeout").to_string(),
                        markup: None,
                    },
                )
                .await?;
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::Active => {
            if let Some(t) = text {
                let rules: Vec<String> = t
                    .split([',', '，', '\n'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if rules.is_empty() {
                    adapter
                        .send_message(
                            target,
                            MessageContent {
                                text: t!("message.warp_input_empty").to_string(),
                                markup: None,
                            },
                        )
                        .await?;
                    return Ok(MessageAction::Handled);
                }

                match ConfigManager::add_warp_routing_rules(rules).await {
                    Ok(_) => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("message.warp_rule_added").to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                    Err(e) => {
                        adapter
                            .send_message(
                                target,
                                MessageContent {
                                    text: t!("message.warp_add_fail", "0" => e.to_string())
                                        .to_string(),
                                    markup: None,
                                },
                            )
                            .await?;
                    }
                }
            }
            return Ok(MessageAction::Handled);
        }
        TimeoutStatus::NotTracked => {}
    }

    Ok(MessageAction::NeedsDestruct)
}
