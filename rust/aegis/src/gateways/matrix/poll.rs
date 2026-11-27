use ruma::events::poll::unstable_start::{
    NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
    UnstablePollStartContentBlock, UnstablePollStartEventContent,
};

use crate::common::markup::render_markup_buttons;
use crate::common::{InlineButton, Markup, MessageContent};

/// A poll to send: one question and 1..=`MAX_POLL_ANSWERS` answers.
pub struct PollSpec {
    pub question: String,
    pub answers: Vec<PollAnswer>,
}

/// A selectable answer. `id` maps back to the button's `data`, so it is never truncated.
pub struct PollAnswer {
    pub id: String,
    pub text: String,
}

/// Equals `ruma_events::poll::start::PollAnswers::MAX_LENGTH`.
pub const MAX_POLL_ANSWERS: usize = 20;
pub const MAX_POLL_BINDINGS: usize = 64;
pub const MAX_ANSWER_LABEL_CHARS: usize = 64;

/// `None` means ruma rejected the answer count; the caller falls back to a numbered list.
pub fn poll_start_content(spec: &PollSpec) -> Option<UnstablePollStartEventContent> {
    let answers: Vec<_> = spec
        .answers
        .iter()
        .map(|a| UnstablePollAnswer::new(a.id.clone(), a.text.clone()))
        .collect();
    let block = UnstablePollStartContentBlock::new(
        spec.question.clone(),
        UnstablePollAnswers::try_from(answers).ok()?,
    );
    Some(NewUnstablePollStartEventContent::plain_text(spec.question.clone(), block).into())
}

/// 返回 (正文, 可选 poll)。有 poll 时正文**不拼**编号清单。
pub fn plan_send(content: &MessageContent) -> (String, Option<PollSpec>) {
    let answers: Vec<PollAnswer> = flatten(&content.markup)
        .into_iter()
        .enumerate()
        .map(|(index, button)| PollAnswer {
            id: index.to_string(),
            text: answer_label(&button.text, &button.data),
        })
        .collect();
    match &content.markup {
        None => (content.text.clone(), None),
        Some(_) if (1..=MAX_POLL_ANSWERS).contains(&answers.len()) => (
            content.text.clone(),
            Some(PollSpec {
                question: rust_i18n::t!("matrix.poll_question").to_string(),
                answers,
            }),
        ),
        Some(markup) => (render_markup_buttons(content.text.clone(), markup), None),
    }
}

/// row-major 拍平。
pub fn flatten(markup: &Option<Markup>) -> Vec<InlineButton> {
    markup
        .as_ref()
        .map(|m| m.buttons.iter().flatten().cloned().collect())
        .unwrap_or_default()
}

fn answer_label(label: &str, data: &str) -> String {
    let label = label.trim();
    let label = if label.is_empty() { data } else { label };
    label.chars().take(MAX_ANSWER_LABEL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{InlineButton, Markup, MessageContent};

    fn content(text: &str, rows: Vec<Vec<(&str, &str)>>) -> MessageContent {
        MessageContent {
            text: text.into(),
            markup: if rows.is_empty() {
                None
            } else {
                Some(Markup {
                    buttons: rows
                        .into_iter()
                        .map(|row| {
                            row.into_iter()
                                .map(|(t, d)| InlineButton {
                                    text: t.into(),
                                    data: d.into(),
                                })
                                .collect()
                        })
                        .collect(),
                })
            },
        }
    }

    #[test]
    fn plan_send_without_markup_keeps_body_and_no_poll() {
        let (body, poll) = plan_send(&content("plain", vec![]));
        assert_eq!(body, "plain");
        assert!(poll.is_none());
    }

    #[test]
    fn plan_send_with_markup_drops_numbered_list_and_builds_poll() {
        let (body, poll) = plan_send(&content(
            "Hi",
            vec![vec![("Search", "/search"), ("Help", "/help")]],
        ));
        assert_eq!(body, "Hi");
        let spec = poll.unwrap();
        assert_eq!(
            spec.answers
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["0", "1"]
        );
        assert_eq!(spec.answers[0].text, "Search");
        assert!(!spec.question.is_empty());
    }

    #[test]
    fn plan_send_flattens_row_major() {
        let (_, poll) = plan_send(&content(
            "t",
            vec![vec![("a", "/a"), ("b", "/b")], vec![("c", "/c")]],
        ));
        let spec = poll.unwrap();
        assert_eq!(
            spec.answers
                .iter()
                .map(|a| a.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn plan_send_with_20_buttons_still_polls() {
        let row: Vec<InlineButton> = (0..20)
            .map(|i| InlineButton {
                text: format!("b{i}"),
                data: format!("/b{i}"),
            })
            .collect();
        let (_, poll) = plan_send(&MessageContent {
            text: "t".into(),
            markup: Some(Markup { buttons: vec![row] }),
        });
        assert_eq!(poll.unwrap().answers.len(), 20);
    }

    #[test]
    fn plan_send_with_21_buttons_falls_back_to_numbered_list() {
        let row: Vec<InlineButton> = (0..21)
            .map(|i| InlineButton {
                text: format!("b{i}"),
                data: format!("/b{i}"),
            })
            .collect();
        let (body, poll) = plan_send(&MessageContent {
            text: "t".into(),
            markup: Some(Markup { buttons: vec![row] }),
        });
        assert!(poll.is_none());
        assert!(body.contains("1. b0"));
        assert!(body.contains("/b0"));
    }

    #[test]
    fn plan_send_truncates_long_label_but_keeps_ids_dense() {
        let long = "x".repeat(200);
        let (_, poll) = plan_send(&content(
            "t",
            vec![vec![(long.as_str(), "/very:long:command:data")]],
        ));
        let spec = poll.unwrap();
        assert_eq!(spec.answers[0].text.chars().count(), MAX_ANSWER_LABEL_CHARS);
        assert_eq!(spec.answers[0].id, "0");
    }

    #[test]
    fn plan_send_uses_data_when_label_is_blank() {
        let (_, poll) = plan_send(&content("t", vec![vec![("   ", "/fallback")]]));
        assert_eq!(poll.unwrap().answers[0].text, "/fallback");
    }

    #[test]
    fn plan_send_truncates_multibyte_label_without_panic() {
        let long = "操作".repeat(60); // 120 chars, 360 bytes
        let (_, poll) = plan_send(&content("t", vec![vec![(long.as_str(), "/x")]]));
        assert_eq!(
            poll.unwrap().answers[0].text.chars().count(),
            MAX_ANSWER_LABEL_CHARS
        );
    }

    #[test]
    fn poll_start_content_serializes_msc3381_shape() {
        let spec = PollSpec {
            question: "选择一个操作".into(),
            answers: vec![
                PollAnswer {
                    id: "0".into(),
                    text: "Search".into(),
                },
                PollAnswer {
                    id: "1".into(),
                    text: "Help".into(),
                },
            ],
        };
        let content = poll_start_content(&spec).expect("2 answers is within limits");
        let json = serde_json::to_value(&content).unwrap();
        let block = &json["org.matrix.msc3381.poll.start"];
        assert_eq!(block["question"]["org.matrix.msc1767.text"], "选择一个操作");
        assert_eq!(block["max_selections"], 1);
        assert_eq!(block["answers"][0]["id"], "0");
        assert_eq!(block["answers"][1]["org.matrix.msc1767.text"], "Help");
        assert_eq!(json["org.matrix.msc1767.text"], "选择一个操作");
    }

    #[test]
    fn poll_start_content_rejects_more_than_max_answers() {
        let spec = PollSpec {
            question: "q".into(),
            answers: (0..21)
                .map(|i| PollAnswer {
                    id: i.to_string(),
                    text: format!("b{i}"),
                })
                .collect(),
        };
        assert!(poll_start_content(&spec).is_none());
    }

    #[test]
    fn poll_start_content_rejects_empty_answers() {
        let spec = PollSpec {
            question: "q".into(),
            answers: vec![],
        };
        assert!(poll_start_content(&spec).is_none());
    }
}
