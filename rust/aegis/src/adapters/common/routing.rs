use crate::adapters::common::{BotAdapter, MessageContent, MessageId, Platform, TargetId};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

pub struct RoutingAdapter {
    primary: Arc<dyn BotAdapter>,
    secondary: Option<Arc<dyn BotAdapter>>,
}

impl RoutingAdapter {
    pub fn new(primary: Arc<dyn BotAdapter>, secondary: Option<Arc<dyn BotAdapter>>) -> Self {
        Self { primary, secondary }
    }
}

pub(crate) fn is_sensitive(text: &str) -> bool {
    const PROTOCOLS: &[&str] = &[
        "vmess://",
        "vless://",
        "trojan://",
        "ss://",
        "hysteria://",
        "hysteria2://",
        "tuic://",
    ];
    const KEY_FIELDS: &[&str] = &["\"privateKey\"", "\"secretKey\"", "\"password\":"];
    // Trailing colon on "password" reduces false-positives like "invalid password"

    PROTOCOLS.iter().any(|p| text.contains(p)) || KEY_FIELDS.iter().any(|k| text.contains(k))
}

#[async_trait]
impl BotAdapter for RoutingAdapter {
    fn platform(&self) -> Platform {
        self.primary.platform()
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        match &self.secondary {
            Some(secondary) if is_sensitive(&content.text) => {
                secondary.send_message(target, content).await
            }
            _ => self.primary.send_message(target, content).await,
        }
    }

    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        self.primary.edit_message(target, msg_id, content).await
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        self.primary.delete_message(target, msg_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_sensitive_detects_vmess() {
        assert!(is_sensitive("vmess://abc123"));
    }

    #[test]
    fn is_sensitive_detects_vless() {
        assert!(is_sensitive("vless://abc123"));
    }

    #[test]
    fn is_sensitive_detects_trojan() {
        assert!(is_sensitive("trojan://abc123"));
    }

    #[test]
    fn is_sensitive_detects_ss() {
        assert!(is_sensitive("ss://abc123"));
    }

    #[test]
    fn is_sensitive_detects_hysteria() {
        assert!(is_sensitive("hysteria://abc123"));
    }

    #[test]
    fn is_sensitive_detects_tuic() {
        assert!(is_sensitive("tuic://abc123"));
    }

    #[test]
    fn is_sensitive_detects_private_key() {
        assert!(is_sensitive("{\"privateKey\":\"abc\"}"));
    }

    #[test]
    fn is_sensitive_detects_secret_key() {
        assert!(is_sensitive("{\"secretKey\":\"abc\"}"));
    }

    #[test]
    fn is_sensitive_rejects_normal_text() {
        assert!(!is_sensitive("系统状态：运行中"));
        assert!(!is_sensitive("Hello world"));
    }

    #[test]
    fn is_sensitive_rejects_empty() {
        assert!(!is_sensitive(""));
    }

    #[test]
    fn is_sensitive_rejects_partial_prefix() {
        assert!(!is_sensitive("vmess is a protocol"));
    }

    #[test]
    fn is_sensitive_detects_hysteria2() {
        assert!(is_sensitive("hysteria2://abc123"));
    }

    mod routing_tests {
        use super::*;
        use crate::adapters::common::MockBotAdapter;

        #[tokio::test]
        async fn sends_sensitive_to_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_send_message().never();

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Matrix);
            secondary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "vless://abc123".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn sends_normal_to_primary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Matrix);
            secondary.expect_send_message().never();

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "normal system message".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn sends_all_to_primary_when_no_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), None);
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "vless://any-content".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }
    }
}
