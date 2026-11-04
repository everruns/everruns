//! Short model view for scheduled runs.
//!
//! A turn started by a session schedule fire does not need the whole session:
//! it needs its task. Sending the full history made a 5-minute poll cost about
//! 380k prompt tokens per fire once its session had grown to 2,349 messages
//! (2026-10-31, background-limits program, part 3b). So when the turn's input
//! is a schedule fire, the model sees:
//!
//! - the leading system messages (the latest compaction checkpoint summary,
//!   when one was restored; the system prompt is added later, outside this
//!   view);
//! - the last [`RECENT_FIRE_MESSAGES`] messages of earlier fires of the same
//!   schedule, as whole fires, so the agent knows what its previous runs found;
//! - the current fire and everything after it in this turn.
//!
//! Everything else (the chat around the schedule, other schedules' fires) is
//! left out of the prompt only. Storage stays lossless, and the fire message
//! tells the model that older messages are not shown.
//!
//! Decision: the window is counted in messages and filled with whole fires,
//! newest first, so a previous run is never cut in the middle. When the newest
//! earlier fire alone is longer than the window, its fire message and its last
//! messages are kept; tool exchanges that lose one side are dropped later by
//! `retain_complete_message_tool_exchanges`.

use crate::builtins::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use crate::builtins::message::{
    MESSAGE_ORIGIN_METADATA_KEY, MessageOrigin, MessageOriginKind, TextContentPart,
};
use std::ops::Range;

/// How many messages of earlier fires of the same schedule a scheduled run sees.
pub(crate) const RECENT_FIRE_MESSAGES: usize = 10;

/// What the current fire message gains in the view, so the model knows why the
/// conversation looks short.
pub(crate) const SCHEDULED_VIEW_NOTE: &str = "[This is a scheduled run. To keep it cheap, you see only the schedule's task, the latest conversation summary if there is one, and the last messages of this schedule's earlier runs. Older messages of the session are stored but not shown here.]";

/// The recurring schedule a message is a fire of, when it is one.
///
/// Read from the reserved origin metadata, which clients cannot set
/// (`strip_reserved_message_metadata`). One-shot fires (no run counter) keep
/// the full history: they fire once, and a reminder usually depends on the
/// conversation that set it.
fn fire_schedule_id(message: &RuntimeMessage) -> Option<String> {
    if message.role != RuntimeMessageRole::User {
        return None;
    }
    let origin = message
        .metadata
        .as_ref()?
        .get(MESSAGE_ORIGIN_METADATA_KEY)?;
    let origin = serde_json::from_value::<MessageOrigin>(origin.clone()).ok()?;
    (origin.kind == MessageOriginKind::Schedule && origin.run.is_some())
        .then_some(origin.source_id)
        .flatten()
}

/// Build the scheduled-run view, or return `messages` unchanged when the
/// turn's input is not a schedule fire or nothing would be left out.
pub(crate) fn apply(messages: Vec<RuntimeMessage>) -> Vec<RuntimeMessage> {
    let Some(current) = messages
        .iter()
        .rposition(|message| message.role == RuntimeMessageRole::User)
    else {
        return messages;
    };
    let Some(schedule_id) = fire_schedule_id(&messages[current]) else {
        return messages;
    };
    let leading = messages[..current]
        .iter()
        .take_while(|message| message.role == RuntimeMessageRole::System)
        .count();

    let kept = earlier_fires(&messages, leading..current, &schedule_id);
    let kept_count: usize = kept.iter().map(Range::len).sum();
    if kept_count == current - leading {
        return messages;
    }
    let dropped = current - leading - kept_count;

    let mut view = Vec::with_capacity(leading + kept_count + messages.len() - current);
    let mut messages = messages.into_iter().enumerate();
    for (index, mut message) in messages.by_ref() {
        if index < leading || kept.iter().any(|range| range.contains(&index)) {
            view.push(message);
        } else if index == current {
            message
                .content
                .push(ContentPart::Text(TextContentPart::new(SCHEDULED_VIEW_NOTE)));
            view.push(message);
            break;
        }
    }
    view.extend(messages.map(|(_, message)| message));
    tracing::info!(
        dropped,
        kept = kept_count,
        "CompactionCapability: scheduled run sees a short model view"
    );
    view
}

/// The index ranges of earlier fires of `schedule_id` within `span` to keep,
/// in order: whole fires, newest first, up to [`RECENT_FIRE_MESSAGES`].
fn earlier_fires(
    messages: &[RuntimeMessage],
    span: Range<usize>,
    schedule_id: &str,
) -> Vec<Range<usize>> {
    let mut fires = Vec::new();
    let mut index = span.start;
    while index < span.end {
        if fire_schedule_id(&messages[index]).as_deref() != Some(schedule_id) {
            index += 1;
            continue;
        }
        let end = messages[index + 1..span.end]
            .iter()
            .position(|message| message.role == RuntimeMessageRole::User)
            .map_or(span.end, |offset| index + 1 + offset);
        fires.push(index..end);
        index = end;
    }

    let mut kept = Vec::new();
    let mut total = 0;
    for fire in fires.into_iter().rev() {
        if total + fire.len() <= RECENT_FIRE_MESSAGES {
            total += fire.len();
            kept.push(fire);
            continue;
        }
        if kept.is_empty() {
            // The newest earlier fire alone is longer than the window: keep
            // its fire message and how it ended.
            kept.push(fire.end - (RECENT_FIRE_MESSAGES - 1)..fire.end);
            kept.push(fire.start..fire.start + 1);
        }
        break;
    }
    kept.reverse();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::message::{MessageOriginRun, ToolCallContentPart, ToolResultContentPart};
    use serde_json::json;

    fn user(text: &str) -> RuntimeMessage {
        RuntimeMessage::user(text)
    }

    fn fire(schedule_id: &str, text: &str) -> RuntimeMessage {
        let mut message = RuntimeMessage::user(text);
        let origin = MessageOrigin::new(MessageOriginKind::Schedule)
            .with_source_id(schedule_id)
            .with_run(MessageOriginRun {
                number: Some(1),
                ..Default::default()
            });
        message.metadata = Some(origin.into_metadata());
        message
    }

    fn agent(text: &str) -> RuntimeMessage {
        RuntimeMessage::assistant(text)
    }

    fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|message| {
                message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    }

    #[test]
    fn a_turn_that_is_not_a_fire_keeps_its_history() {
        let messages = vec![fire("s1", "poll"), agent("pending"), user("hi")];
        assert_eq!(texts(&apply(messages.clone())), texts(&messages));
    }

    #[test]
    fn a_fire_sees_the_summary_its_earlier_fires_and_itself() {
        let messages = vec![
            RuntimeMessage::system("[CONVERSATION_SUMMARY]s[/CONVERSATION_SUMMARY]"),
            user("please poll the claim every 5 minutes"),
            agent("scheduled"),
            fire("s1", "poll 1"),
            agent("pending 1"),
            user("unrelated question"),
            agent("unrelated answer"),
            fire("other", "other schedule"),
            agent("other result"),
            fire("s1", "poll 2"),
            agent("pending 2"),
            fire("s1", "poll 3"),
        ];
        let view = texts(&apply(messages));
        assert_eq!(
            view,
            vec![
                "[CONVERSATION_SUMMARY]s[/CONVERSATION_SUMMARY]".to_string(),
                "poll 1".to_string(),
                "pending 1".to_string(),
                "poll 2".to_string(),
                "pending 2".to_string(),
                format!("poll 3|{SCHEDULED_VIEW_NOTE}"),
            ]
        );
    }

    #[test]
    fn the_window_keeps_whole_recent_fires_and_the_current_turn() {
        let mut messages = Vec::new();
        for run in 1..=8 {
            messages.push(fire("s1", &format!("poll {run}")));
            messages.push(agent(&format!("checking {run}")));
            messages.push(agent(&format!("pending {run}")));
        }
        messages.push(fire("s1", "poll 9"));
        messages.push(agent("still working"));
        let view = texts(&apply(messages));
        // 3 messages per fire: three whole fires fit in 10.
        assert_eq!(view.len(), 9 + 2);
        assert_eq!(view[0], "poll 6");
        assert_eq!(view[9], format!("poll 9|{SCHEDULED_VIEW_NOTE}"));
        assert_eq!(view[10], "still working");
    }

    #[test]
    fn a_long_previous_fire_keeps_its_fire_message_and_its_end() {
        let mut messages = vec![fire("s1", "poll 1")];
        for step in 1..=20 {
            messages.push(agent(&format!("step {step}")));
        }
        messages.push(fire("s1", "poll 2"));
        let view = texts(&apply(messages));
        assert_eq!(view.len(), RECENT_FIRE_MESSAGES + 1);
        assert_eq!(view[0], "poll 1");
        assert_eq!(view[1], "step 12");
        assert_eq!(view[9], "step 20");
    }

    #[test]
    fn one_shot_and_client_labelled_messages_keep_the_full_history() {
        let mut one_shot = RuntimeMessage::user("remind me");
        one_shot.metadata = Some(
            MessageOrigin::new(MessageOriginKind::Schedule)
                .with_source_id("s1")
                .into_metadata(),
        );
        let mut labelled = RuntimeMessage::user("poll");
        labelled.metadata = Some(
            [
                ("source".to_string(), json!("schedule")),
                ("schedule_id".to_string(), json!("s1")),
            ]
            .into_iter()
            .collect(),
        );
        for input in [one_shot, labelled] {
            let messages = vec![user("the reminder's context"), agent("noted"), input];
            assert_eq!(texts(&apply(messages.clone())), texts(&messages));
        }
    }

    #[test]
    fn a_short_session_is_left_alone() {
        let messages = vec![fire("s1", "poll 1"), agent("pending"), fire("s1", "poll 2")];
        assert_eq!(texts(&apply(messages.clone())), texts(&messages));
    }

    #[test]
    fn tool_exchanges_inside_kept_fires_stay_whole() {
        let mut call = RuntimeMessage::assistant("");
        call.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
            "call_1",
            "web_fetch",
            json!({"url": "https://claims.example"}),
        ))];
        let mut result = RuntimeMessage::user("");
        result.role = RuntimeMessageRole::ToolResult;
        result.content = vec![ContentPart::ToolResult(ToolResultContentPart::new(
            "call_1",
            Some(json!({"status": "pending"})),
            None,
        ))];
        let messages = vec![
            user("chat"),
            fire("s1", "poll 1"),
            call,
            result,
            agent("pending"),
            fire("s1", "poll 2"),
        ];
        let view = apply(messages);
        assert_eq!(view.len(), 5);
        assert_eq!(view[1].role, RuntimeMessageRole::Agent);
        assert_eq!(view[2].role, RuntimeMessageRole::ToolResult);
    }
}
