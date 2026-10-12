//! Recorded tool changes, replayed from the transcript before each request.
//!
//! Like Astra effort snapshots (`reasoning_updates`), entries live on
//! completed assistant messages, so the immutable transcript is the record across worker restarts
//! and workers. An entry describes what changed before the generation that
//! produced its message: the first one of an epoch carries the baseline, later
//! ones the difference between the declared tools and the tools wanted then.
//!
//! An epoch ends wherever the cached prefix restarts anyway: a message from
//! another model, a transcript cut by a compaction checkpoint (its stored
//! messages are gone, so no baseline is found), or a session from before this
//! existed. The next request then records a new baseline.
//!
//! Drivers that render changes in a cache-safe form send the epoch's baseline
//! as their tool list and the transitions inline; the others ignore both, since
//! `tools` still carries the wanted set. The baseline is offered only when every
//! change of the epoch reaches the request (see [`ToolChangeReplay::baseline_for`]).

// Design of record: knowledge/execution/cache-stable-tool-changes.md.
use crate::engine::message::{RuntimeMessage, RuntimeMessageRole};
use crate::engine::tool_types::ToolDefinition;
use crate::engine::typed_id::MessageId;
use everruns_contracts::driver_registry::Message;
use everruns_contracts::tool_changes::{DeclaredTools, ToolChanges};
use std::collections::HashMap;

pub(super) const KEY: &str = everruns_contracts::tool_changes::METADATA_KEY;

#[derive(Debug, Default)]
pub(super) struct ToolChangeReplay {
    /// Changes keyed by the message they take effect immediately before.
    pub transitions: HashMap<MessageId, ToolChanges>,
    /// Entry to store on this generation's assistant message, if any.
    pub record: Option<ToolChanges>,
    /// Tools fixed at the start of the epoch.
    baseline: Option<Vec<ToolDefinition>>,
    /// This request's change has no message to ride on.
    unanchored: bool,
}

impl ToolChangeReplay {
    /// Store the entry on the assistant message's metadata.
    pub(super) fn stamp(&self, metadata: &mut HashMap<String, serde_json::Value>) {
        if let Some(record) = &self.record {
            metadata.insert(KEY.to_string(), serde_json::json!(record));
        }
    }

    /// The epoch's baseline, when every change since it is on a message of
    /// `sent`. A change dropped from the request (a model view or trimming cut
    /// its message) would leave the model unaware of a tool, so drivers then
    /// get no baseline and send the wanted set as before.
    pub(super) fn baseline_for(&self, sent: &[Message]) -> Option<Vec<ToolDefinition>> {
        let carried = sent.iter().filter(|m| m.tool_changes.is_some()).count();
        (!self.unanchored && carried == self.transitions.len())
            .then(|| self.baseline.clone())
            .flatten()
    }
}

fn entry(message: &RuntimeMessage) -> Option<ToolChanges> {
    let value = message.metadata.as_ref()?.get(KEY)?;
    serde_json::from_value(value.clone()).ok()
}

fn same_model(message: &RuntimeMessage, provider: &str, model: &str) -> bool {
    let field = |key| {
        message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(key))
            .and_then(serde_json::Value::as_str)
    };
    field("provider") == Some(provider) && field("model") == Some(model)
}

/// The first non-system message from `start` through `end`: where a change
/// recorded on the assistant message at `end` took effect.
fn anchor(messages: &[&RuntimeMessage], start: usize, end: usize) -> Option<MessageId> {
    messages
        .get(start..=end)?
        .iter()
        .find(|message| message.role != RuntimeMessageRole::System)
        .map(|message| message.id)
}

pub(super) fn prepare(
    messages: &[RuntimeMessage],
    provider: &str,
    model: &str,
    wanted: &[ToolDefinition],
) -> ToolChangeReplay {
    let messages: Vec<_> = messages
        .iter()
        .filter(|message| !super::is_error_placeholder_message(message))
        .collect();
    let mut declared: Option<DeclaredTools> = None;
    let mut baseline = None;
    let mut transitions = HashMap::new();
    let mut input_start = 0;
    for (index, message) in messages.iter().enumerate() {
        if message.role != RuntimeMessageRole::Agent {
            continue;
        }
        if !same_model(message, provider, model) {
            declared = None;
            transitions.clear();
        } else if let Some(changes) = entry(message) {
            if changes.baseline.is_some() {
                declared = Some(DeclaredTools::default());
                baseline.clone_from(&changes.baseline);
                transitions.clear();
            }
            if let Some(declared) = declared.as_mut() {
                declared.apply(&changes);
                if !changes.is_empty()
                    && let Some(at) = anchor(&messages, input_start, index)
                {
                    transitions.insert(at, changes);
                }
            }
        }
        input_start = index + 1;
    }

    let Some(declared) = declared else {
        return ToolChangeReplay {
            record: Some(ToolChanges::baseline(wanted.to_vec())),
            baseline: Some(wanted.to_vec()),
            ..ToolChangeReplay::default()
        };
    };
    let changes = declared.diff(wanted);
    if changes.is_empty() {
        return ToolChangeReplay {
            transitions,
            baseline,
            ..ToolChangeReplay::default()
        };
    }
    // With no new input since the last answer, the change precedes the output
    // being generated now; its replay anchors it there next time.
    let at = anchor(&messages, input_start, messages.len().saturating_sub(1));
    if let Some(at) = at {
        transitions.insert(at, changes.clone());
    }
    ToolChangeReplay {
        transitions,
        record: Some(changes),
        baseline,
        unanchored: at.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER: &str = "anthropic";
    const MODEL: &str = "claude-sonnet-5-5";

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::function(name, description, json!({"type": "object"}))
    }

    fn answer(model: &str, changes: Option<&ToolChanges>) -> RuntimeMessage {
        let mut message = RuntimeMessage::assistant("done");
        let mut metadata = HashMap::from([
            ("provider".to_string(), json!(PROVIDER)),
            ("model".to_string(), json!(model)),
        ]);
        if let Some(changes) = changes {
            metadata.insert(KEY.to_string(), json!(changes));
        }
        message.metadata = Some(metadata);
        message
    }

    /// Runs `prepare` and appends the answer it would store.
    fn step(
        messages: &mut Vec<RuntimeMessage>,
        input: &str,
        wanted: &[ToolDefinition],
    ) -> ToolChangeReplay {
        messages.push(RuntimeMessage::user(input));
        let replay = prepare(messages, PROVIDER, MODEL, wanted);
        messages.push(answer(MODEL, replay.record.as_ref()));
        replay
    }

    #[test]
    fn the_first_request_records_a_baseline_and_nothing_else() {
        let mut messages = Vec::new();
        let replay = step(&mut messages, "hi", &[tool("a", "A")]);
        let record = replay.record.expect("baseline recorded");
        assert_eq!(record.baseline.as_ref().map(Vec::len), Some(1));
        assert!(record.is_empty());
        assert!(replay.transitions.is_empty());

        let replay = step(&mut messages, "again", &[tool("a", "A")]);
        assert!(replay.record.is_none());
        assert!(replay.transitions.is_empty());
    }

    #[test]
    fn a_change_is_recorded_once_and_replayed_at_its_input() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        let wanted = [tool("a", "A"), tool("b", "B")];
        let replay = step(&mut messages, "reveal b", &wanted);
        let input = messages[2].id;
        let record = replay.record.expect("change recorded");
        assert!(record.baseline.is_none());
        assert_eq!(record.added[0].name(), "b");
        assert_eq!(replay.transitions[&input].added[0].name(), "b");

        // Later requests replay the stored change at the same input and
        // record nothing new.
        let replay = step(&mut messages, "next", &wanted);
        assert!(replay.record.is_none());
        assert_eq!(replay.transitions.len(), 1);
        assert_eq!(replay.transitions[&input].added[0].name(), "b");
    }

    #[test]
    fn a_removed_tool_is_announced_once() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A"), tool("b", "B")]);
        let replay = step(&mut messages, "drop b", &[tool("a", "A")]);
        assert_eq!(replay.record.expect("removal").removed, ["b"]);
        assert!(
            step(&mut messages, "next", &[tool("a", "A")])
                .record
                .is_none()
        );
    }

    #[test]
    fn another_model_starts_a_new_epoch() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        step(&mut messages, "reveal", &[tool("a", "A"), tool("b", "B")]);
        messages.push(RuntimeMessage::user("switch"));
        messages.push(answer("claude-opus-5-5", None));

        let replay = step(&mut messages, "back", &[tool("a", "A"), tool("b", "B")]);
        let record = replay.record.expect("new baseline");
        assert_eq!(record.baseline.as_ref().map(Vec::len), Some(2));
        assert!(replay.transitions.is_empty());
    }

    #[test]
    fn the_baseline_is_offered_only_when_every_change_is_sent() {
        use everruns_contracts::driver_registry::MessageRole;
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        let wanted = [tool("a", "A"), tool("b", "B")];
        let replay = step(&mut messages, "reveal b", &wanted);
        let sent = |with_change: bool| {
            let mut input = Message::text(MessageRole::User, "reveal b");
            input.tool_changes = with_change.then(|| replay.transitions[&messages[2].id].clone());
            vec![Message::text(MessageRole::User, "hi"), input]
        };
        let baseline = replay.baseline_for(&sent(true)).expect("all changes sent");
        assert_eq!(baseline.len(), 1, "the epoch's first tool list");
        assert!(replay.baseline_for(&sent(false)).is_none());

        // A change with no message to ride on (no input since the last
        // answer) cannot be sent inline either.
        let replay = prepare(&messages, PROVIDER, MODEL, &[tool("a", "A")]);
        assert!(replay.record.is_some());
        assert!(replay.baseline_for(&sent(true)).is_none());
    }

    #[test]
    fn a_transcript_without_entries_starts_with_a_baseline() {
        let mut messages = vec![RuntimeMessage::user("old"), answer(MODEL, None)];
        let replay = step(&mut messages, "new", &[tool("a", "A")]);
        assert!(replay.record.expect("baseline").baseline.is_some());
    }
}
