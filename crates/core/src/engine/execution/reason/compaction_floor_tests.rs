use super::trim_floor;
use crate::engine::message::RuntimeMessage;
use std::collections::HashMap;

/// `turns` user/assistant pairs of ~100 tokens each, sequences 1, 2, ...
fn history(
    turns: usize,
) -> (
    Vec<RuntimeMessage>,
    HashMap<crate::engine::typed_id::MessageId, i64>,
) {
    let text = "word ".repeat(80);
    let mut messages = Vec::new();
    for _ in 0..turns {
        messages.push(RuntimeMessage::user(text.clone()));
        messages.push(RuntimeMessage::assistant(text.clone()));
    }
    let sequences = messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id, index as i64 + 1))
        .collect();
    (messages, sequences)
}

#[test]
fn short_history_keeps_no_floor() {
    let (messages, sequences) = history(10);
    assert_eq!(trim_floor(&messages, &sequences, 1_000, None), None);
}

#[test]
fn long_history_floors_before_a_user_message_two_budgets_back() {
    let (messages, sequences) = history(40);
    let floor = trim_floor(&messages, &sequences, 1_000, None).expect("floor");
    // The first kept message is a user message (odd sequence).
    assert_eq!((floor + 1) % 2, 1);
    let kept = &messages[floor as usize..];
    assert!(kept.len() >= 18, "keeps about two budgets: {}", kept.len());
    // A floor that is not past the current one is not rewritten.
    assert_eq!(trim_floor(&messages, &sequences, 1_000, Some(floor)), None);
}

#[test]
fn a_kept_message_stored_before_the_floor_blocks_it() {
    let (mut messages, mut sequences) = history(40);
    let early = RuntimeMessage::assistant("late tool result");
    sequences.insert(early.id, 0);
    messages.push(early);
    assert_eq!(trim_floor(&messages, &sequences, 1_000, None), None);
}
