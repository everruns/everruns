//! Where dynamic `<facts>` blocks go in the request.
//!
//! A request ends on an input: a user message or a batch of tool results. The
//! runtime adds the live facts (such as the current time) right after that
//! input, and the model answers. On the next request that answer is history,
//! and the facts block before it has to come back unchanged. Dropping or
//! rebuilding it rewrites the conversation the answer was produced from, which
//! restarts the provider's prompt cache and invalidates thinking blocks that
//! Claude binds to their conversation.
//!
//! So a block goes after every input that an answer followed, rendered as of
//! that input's own timestamp. The input is stored history with a fixed
//! timestamp, so every later request renders the same text in the same place.

use chrono::{DateTime, Utc};

use crate::message::{RuntimeMessage, RuntimeMessageRole};

/// Insert a facts block after each input the model answered or is about to
/// answer. `render` returns the block as of a moment, or `None` when no active
/// capability contributes dynamic facts.
///
/// Returns the messages and how many trailing messages will not be replayed
/// as-is on the next request (`LlmCallConfig::volatile_suffix_len`). That is
/// zero except when the conversation ends on an answer rather than an input
/// (a continuation): the block then trails the answer, as of now, and nothing
/// reproduces it later.
pub(super) fn interleave_facts(
    messages: Vec<RuntimeMessage>,
    render: impl Fn(DateTime<Utc>) -> Option<String>,
) -> (Vec<RuntimeMessage>, usize) {
    let answered =
        |next: Option<&RuntimeMessageRole>| matches!(next, None | Some(RuntimeMessageRole::Agent));
    let roles: Vec<RuntimeMessageRole> = messages.iter().map(|m| m.role.clone()).collect();
    let ends_on_answer = matches!(
        roles
            .iter()
            .rev()
            .find(|r| **r != RuntimeMessageRole::System),
        Some(RuntimeMessageRole::Agent)
    );
    let mut placed = Vec::with_capacity(messages.len() + 1);
    for (index, message) in messages.into_iter().enumerate() {
        let input_at = matches!(
            message.role,
            RuntimeMessageRole::User | RuntimeMessageRole::ToolResult
        )
        .then_some(message.created_at);
        placed.push(message);
        let next = roles[index + 1..]
            .iter()
            .find(|r| **r != RuntimeMessageRole::System);
        if let Some(at) = input_at
            && answered(next)
            && let Some(block) = render(at)
        {
            placed.push(RuntimeMessage::user(block));
        }
    }
    if ends_on_answer && let Some(block) = render(Utc::now()) {
        placed.push(RuntimeMessage::user(block));
        return (placed, 1);
    }
    (placed, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 10, minute, 0).unwrap()
    }

    fn message(role: RuntimeMessageRole, text: &str, minute: u32) -> RuntimeMessage {
        let mut message = match role {
            RuntimeMessageRole::User => RuntimeMessage::user(text),
            RuntimeMessageRole::Agent => RuntimeMessage::assistant(text),
            RuntimeMessageRole::System => RuntimeMessage::system(text),
            RuntimeMessageRole::ToolResult => RuntimeMessage::user(text),
        };
        message.role = role;
        message.created_at = at(minute);
        message
    }

    fn render(moment: DateTime<Utc>) -> Option<String> {
        Some(format!("<facts>{}</facts>", moment.format("%H:%M")))
    }

    fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|m| m.text().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn a_block_follows_every_answered_input_as_of_that_input() {
        use RuntimeMessageRole::*;
        let history = vec![
            message(User, "task", 0),
            message(Agent, "call tools", 1),
            message(ToolResult, "result a", 2),
            message(ToolResult, "result b", 3),
            message(System, "loop warning", 4),
            message(Agent, "done", 5),
            message(User, "next", 6),
        ];
        let (placed, volatile) = interleave_facts(history, render);
        assert_eq!(
            texts(&placed),
            [
                "task",
                "<facts>10:00</facts>",
                "call tools",
                "result a",
                // One block per tool batch, after its last result.
                "result b",
                "<facts>10:03</facts>",
                "loop warning",
                "done",
                "next",
                "<facts>10:06</facts>",
            ]
        );
        assert_eq!(volatile, 0);
    }

    /// The property the placement exists for: the next request replays this
    /// request's messages unchanged and only appends.
    #[test]
    fn the_next_request_only_appends() {
        use RuntimeMessageRole::*;
        let turn_one = vec![message(User, "task", 0)];
        let mut turn_two = turn_one.clone();
        turn_two.extend([
            message(Agent, "call tool", 1),
            message(ToolResult, "result", 2),
        ]);
        let (one, _) = interleave_facts(turn_one, render);
        let (two, _) = interleave_facts(turn_two, render);
        assert_eq!(texts(&two)[..one.len()], texts(&one)[..]);
        assert_eq!(texts(&two).len(), one.len() + 3);
    }

    #[test]
    fn a_continuation_gets_a_trailing_block_marked_volatile() {
        use RuntimeMessageRole::*;
        let history = vec![message(User, "task", 0), message(Agent, "partial", 1)];
        let (placed, volatile) = interleave_facts(history, render);
        assert_eq!(
            texts(&placed)[..3],
            ["task", "<facts>10:00</facts>", "partial"]
        );
        assert_eq!(placed.len(), 4);
        assert_eq!(volatile, 1);
    }

    #[test]
    fn no_dynamic_facts_leaves_the_messages_alone() {
        use RuntimeMessageRole::*;
        let history = vec![message(User, "task", 0), message(Agent, "answer", 1)];
        let (placed, volatile) = interleave_facts(history, |_| None);
        assert_eq!(texts(&placed), ["task", "answer"]);
        assert_eq!(volatile, 0);
    }
}
