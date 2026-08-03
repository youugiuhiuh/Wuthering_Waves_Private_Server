use crate::app::interaction::{ConversationId, OutputAction, OutputPayload, Sensitivity};
use crate::common::{InlineButton, Markup, MessageContent};
use crate::core::system::log_audit::{LogAudit, SERVICE_SING_BOX, SERVICE_WWPS_CORE};
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use crate::utils;
use rust_i18n::t;

pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    match data {
        "m_log" => {
            let markup = Markup {
                buttons: vec![
                    vec![
                        InlineButton {
                            text: t!("log.xray_btn").into(),
                            data: "l_xray".into(),
                        },
                        InlineButton {
                            text: t!("log.box_btn").into(),
                            data: "l_box".into(),
                        },
                    ],
                    vec![InlineButton {
                        text: t!("menu.back_ops").into(),
                        data: "m_ops_center".into(),
                    }],
                ],
            };
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: format!("{}\n{}", t!("menu.log_audit"), t!("menu.log_audit_desc")),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "l_xray" => {
            let status = LogAudit::service_status(SERVICE_WWPS_CORE).await;
            let status_icon = if status.active { "🟢" } else { "🔴" };
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("log.view_tail").into(),
                        data: "l_xray_tail".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.refresh").into(),
                        data: "l_xray".into(),
                    }],
                    vec![InlineButton {
                        text: t!("log.back_log").into(),
                        data: "m_log".into(),
                    }],
                ],
            };
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "log.xray_log_title",
                            "0" => status_icon,
                            "1" => status.status_text,
                            "2" => SERVICE_WWPS_CORE
                        )
                        .into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "l_box" => {
            let status = LogAudit::service_status(SERVICE_SING_BOX).await;
            let status_icon = if status.active { "🟢" } else { "🔴" };
            let markup = Markup {
                buttons: vec![
                    vec![InlineButton {
                        text: t!("log.view_tail").into(),
                        data: "l_box_tail".into(),
                    }],
                    vec![InlineButton {
                        text: t!("menu.refresh").into(),
                        data: "l_box".into(),
                    }],
                    vec![InlineButton {
                        text: t!("log.back_log").into(),
                        data: "m_log".into(),
                    }],
                ],
            };
            event
                .output
                .as_adapter()
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "log.box_log_title",
                            "0" => status_icon,
                            "1" => status.status_text,
                            "2" => SERVICE_SING_BOX
                        )
                        .into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;
        }
        "l_xray_tail" => {
            event
                .output
                .as_adapter()
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("log.fetching_xray").into()),
                )
                .await?;
            let output = event.output.clone();
            let target = event.target.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_WWPS_CORE, 50).await {
                    let _ = output
                        .publish(OutputAction::SendText {
                            target_conversation: ConversationId::new(target.0.clone()).unwrap(),
                            payload: OutputPayload::Text {
                                text: t!("log.xray_tail_title", "0" => utils::escape_html(&log))
                                    .into_owned(),
                            },
                            sensitivity: Sensitivity::Public,
                        })
                        .await;
                }
            });
        }
        "l_box_tail" => {
            event
                .output
                .as_adapter()
                .answer_callback(
                    &event.target,
                    &event.callback_id,
                    Some(t!("log.fetching_box").into()),
                )
                .await?;
            let output = event.output.clone();
            let target = event.target.clone();
            tokio::spawn(async move {
                if let Ok(log) = LogAudit::tail_logs(SERVICE_SING_BOX, 50).await {
                    let _ = output
                        .publish(OutputAction::SendText {
                            target_conversation: ConversationId::new(target.0.clone()).unwrap(),
                            payload: OutputPayload::Text {
                                text: t!("log.box_tail_title", "0" => utils::escape_html(&log))
                                    .into_owned(),
                            },
                            sensitivity: Sensitivity::Public,
                        })
                        .await;
                }
            });
        }
        _ => {}
    }
    Ok(HandlerAction::Done)
}
