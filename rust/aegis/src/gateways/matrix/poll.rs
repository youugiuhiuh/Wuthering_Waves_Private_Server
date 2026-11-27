use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use ruma::events::poll::unstable_start::{
    NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
    UnstablePollStartContentBlock, UnstablePollStartEventContent,
};

use crate::common::markup::render_markup_buttons;
use crate::common::{InlineButton, Markup, MessageContent, MessageId};

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
            text: answer_label(&button.text),
        })
        .collect();
    match &content.markup {
        None => (content.text.clone(), None),
        // 空 label 时不能落回 data：那会把内部命令串暴露成可点选项。
        Some(_)
            if (1..=MAX_POLL_ANSWERS).contains(&answers.len())
                && answers.iter().all(|a| !a.text.is_empty()) =>
        {
            (
                content.text.clone(),
                Some(PollSpec {
                    question: rust_i18n::t!("matrix.poll_question").to_string(),
                    answers,
                }),
            )
        }
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

fn answer_label(label: &str) -> String {
    label.trim().chars().take(MAX_ANSWER_LABEL_CHARS).collect()
}

/// 一次发送的 poll 与其按钮：`buttons` 按 row-major 拍平，下标即 poll answer id。
pub struct PollBinding {
    /// 承载正文的 `m.room.message` event id；派发时作为 `CallbackEvent::msg_id`。
    pub anchor: MessageId,
    pub buttons: Vec<InlineButton>,
    created: Instant,
    answered: bool,
    /// 插入次序，由 `PollStore` 分配；`created` 并列时保证淘汰确定。
    seq: u64,
}

impl PollBinding {
    pub fn new(anchor: MessageId, buttons: Vec<InlineButton>) -> Self {
        Self {
            anchor,
            buttons,
            created: Instant::now(),
            answered: false,
            seq: 0,
        }
    }
}

/// poll event id → binding。所有方法同步：调用方不得持锁跨 `.await`。
#[derive(Default)]
pub struct PollStore {
    bindings: Mutex<HashMap<String, PollBinding>>,
    next_seq: AtomicU64,
}

impl PollStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 内存表没有跨调用的不变量，锁中毒时取回内部值比 panic 更合适。
    fn lock(&self) -> MutexGuard<'_, HashMap<String, PollBinding>> {
        self.bindings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 超出 `MAX_POLL_BINDINGS` 时按 `(created, seq)` 逐个淘汰最旧。
    pub fn insert(&self, poll_event_id: String, mut binding: PollBinding) {
        binding.seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let mut map = self.lock();
        map.insert(poll_event_id, binding);
        while map.len() > MAX_POLL_BINDINGS {
            let oldest = map
                .iter()
                .min_by_key(|(_, b)| (b.created, b.seq))
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => map.remove(&key),
                None => break,
            };
        }
    }

    pub fn find_by_anchor(&self, anchor: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|(_, b)| b.anchor.0 == anchor)
            .map(|(k, _)| k.clone())
    }

    /// 找到并移除某条 anchor（正文消息）名下的 poll，返回它的 poll event id。
    pub fn take_by_anchor(&self, anchor: &str) -> Option<String> {
        let mut map = self.lock();
        let key = map
            .iter()
            .find(|(_, b)| b.anchor.0 == anchor)
            .map(|(k, _)| k.clone())?;
        map.remove(&key);
        Some(key)
    }

    /// 返回 `(anchor, button.data)`；未知 poll / 已 answered / 下标越界 → `None`。
    pub fn take_unanswered(
        &self,
        poll_event_id: &str,
        index: usize,
    ) -> Option<(MessageId, String)> {
        let mut map = self.lock();
        let binding = map.get_mut(poll_event_id)?;
        if binding.answered {
            return None;
        }
        // 越界是调用方的错，不能因此把这条 poll 标成已消费。
        let data = binding.buttons.get(index)?.data.clone();
        binding.answered = true;
        Some((binding.anchor.clone(), data))
    }
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
    fn plan_send_falls_back_when_a_label_is_blank() {
        let (body, poll) = plan_send(&content("t", vec![vec![("   ", "/fallback")]]));
        assert!(
            poll.is_none(),
            "空 label 不得产出 poll（会把命令串暴露成选项）"
        );
        assert!(body.contains("/fallback"), "回退清单里应包含该命令");
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

    fn button(t: &str, d: &str) -> InlineButton {
        InlineButton {
            text: t.into(),
            data: d.into(),
        }
    }

    fn binding(anchor: &str, buttons: Vec<InlineButton>) -> PollBinding {
        PollBinding::new(MessageId(anchor.into()), buttons)
    }

    #[test]
    fn take_unanswered_returns_anchor_and_data_once() {
        let s = PollStore::new();
        s.insert(
            "$poll".into(),
            binding("$anchor", vec![button("Search", "/search")]),
        );
        assert_eq!(
            s.take_unanswered("$poll", 0).map(|(m, d)| (m.0, d)),
            Some(("$anchor".into(), "/search".into()))
        );
        assert!(
            s.take_unanswered("$poll", 0).is_none(),
            "第二次不得再派发（连点/改票）"
        );
    }

    #[test]
    fn take_unanswered_ignores_unknown_poll_and_out_of_range() {
        let s = PollStore::new();
        assert!(s.take_unanswered("$nope", 0).is_none());
        s.insert("$poll".into(), binding("$anchor", vec![button("a", "/a")]));
        assert!(s.take_unanswered("$poll", 5).is_none());
        assert_eq!(
            s.take_unanswered("$poll", 0).map(|(_, d)| d),
            Some("/a".into()),
            "越界失败不得把 answered 置位"
        );
    }

    #[test]
    fn find_by_anchor_and_take() {
        let s = PollStore::new();
        s.insert("$p1".into(), binding("$a1", vec![button("a", "/a")]));
        assert_eq!(s.find_by_anchor("$a1").as_deref(), Some("$p1"));
        assert!(s.find_by_anchor("$a2").is_none());
        assert_eq!(s.take_by_anchor("$a1").as_deref(), Some("$p1"));
        assert!(s.find_by_anchor("$a1").is_none());
    }

    #[test]
    fn take_by_anchor_returns_and_removes_old_poll_once() {
        let s = PollStore::new();
        s.insert("$old".into(), binding("$anchor", vec![button("a", "/a")]));
        assert_eq!(s.take_by_anchor("$anchor").as_deref(), Some("$old"));
        assert!(
            s.take_by_anchor("$anchor").is_none(),
            "同一个 anchor 只能取一次"
        );
    }

    #[test]
    fn take_by_anchor_leaves_other_anchors_alone() {
        let s = PollStore::new();
        s.insert("$p1".into(), binding("$a1", vec![button("a", "/a")]));
        s.insert("$p2".into(), binding("$a2", vec![button("b", "/b")]));
        assert_eq!(s.take_by_anchor("$a1").as_deref(), Some("$p1"));
        assert_eq!(s.find_by_anchor("$a2").as_deref(), Some("$p2"));
        assert!(s.take_by_anchor("$unknown").is_none());
    }

    #[test]
    fn insert_evicts_oldest_beyond_capacity() {
        let s = PollStore::new();
        for i in 0..=MAX_POLL_BINDINGS {
            s.insert(
                format!("$p{i}"),
                binding(&format!("$a{i}"), vec![button("a", "/a")]),
            );
        }
        assert_eq!(s.find_by_anchor("$a0"), None, "最旧的必须被淘汰");
        assert!(
            s.find_by_anchor(&format!("$a{MAX_POLL_BINDINGS}"))
                .is_some()
        );
    }
}
