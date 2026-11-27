use ruma::events::poll::unstable_start::{
    NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
    UnstablePollStartContentBlock, UnstablePollStartEventContent,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
