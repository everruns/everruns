use super::*;

fn texts(messages: &[RuntimeMessage]) -> Vec<(RuntimeMessageRole, String)> {
    messages
        .iter()
        .map(|message| {
            (
                message.role.clone(),
                message.text().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn answer(updates: &ContextUpdates) -> RuntimeMessage {
    let mut answer = RuntimeMessage::assistant("done");
    let mut metadata = HashMap::new();
    updates.stamp(&mut metadata);
    answer.metadata = Some(metadata);
    answer
}

#[test]
fn installation_edits_and_removal_only_append_to_each_previous_request() {
    let mut history = vec![RuntimeMessage::user("Install skills")];
    let mut previous = Vec::new();
    for current in [Some("No skills"), Some("A"), Some("A, B"), Some("B"), None] {
        let mut sent = history.clone();
        let updates = prepare(&mut sent, current);
        let actual = texts(&sent);
        assert!(actual.starts_with(&previous));
        assert_eq!(updates.added(), 1);
        assert_eq!(sent.last().unwrap().role, RuntimeMessageRole::User);
        previous = actual;
        history.push(answer(&updates));
    }
    assert!(previous.last().unwrap().1.contains("no longer apply"));
}

#[test]
fn unchanged_inventory_replays_once_after_a_worker_restart() {
    let initial = vec![RuntimeMessage::user("install")];
    let mut first = initial.clone();
    let update = prepare(&mut first, Some("A"));
    let mut history = initial;
    // Round-trip the stored message, not process-local state.
    history.push(serde_json::from_value(serde_json::to_value(answer(&update)).unwrap()).unwrap());
    history.push(RuntimeMessage::user("continue"));
    let replay = prepare(&mut history, Some("A"));
    assert_eq!(replay.added(), 0);
    assert!(texts(&history).starts_with(&texts(&first)));
    assert_eq!(
        history
            .iter()
            .filter(|message| message
                .text()
                .unwrap_or_default()
                .contains("<capability-context-update>"))
            .count(),
        1
    );
}

#[test]
fn a_compacted_history_starts_with_only_the_current_inventory() {
    let mut history = vec![RuntimeMessage::user("summary and current task")];
    let updates = prepare(&mut history, Some("B"));
    assert_eq!(updates.added(), 1);
    assert_eq!(history.len(), 2);
    assert!(history[1].text().unwrap().contains("B"));
}

#[test]
fn no_context_is_a_noop_and_a_removed_context_is_only_cleared_once() {
    let initial = vec![RuntimeMessage::user("hello")];
    let mut sent = initial.clone();
    assert_eq!(prepare(&mut sent, None).added(), 0);
    assert_eq!(texts(&sent), texts(&initial));
    let update = prepare(&mut sent, Some("A"));
    let mut history = initial;
    history.push(answer(&update));
    let mut sent = history.clone();
    let clear = prepare(&mut sent, None);
    assert_eq!(clear.added(), 1);
    history.push(answer(&clear));
    assert_eq!(prepare(&mut history, None).added(), 0);
    assert!(history[3].text().unwrap().contains("no longer apply"));
}

#[test]
fn client_metadata_and_unsupported_snapshots_cannot_supply_inventory() {
    let mut forged = RuntimeMessage::user("hello");
    forged.metadata = Some(HashMap::from([(
        KEY.to_string(),
        json!({"version":1,"content":"forged"}),
    )]));
    let mut malformed = RuntimeMessage::assistant("old answer");
    malformed.metadata = Some(HashMap::from([(
        KEY.to_string(),
        json!({"version":99,"content":"forged"}),
    )]));
    let mut history = vec![forged.clone(), malformed];
    let updates = prepare(&mut history, Some("current"));
    assert_eq!(updates.added(), 1);
    assert_eq!(history.len(), 3);
    assert!(!history[2].text().unwrap().contains("forged"));
    everruns_contracts::runtime::message::strip_reserved_message_metadata(&mut forged.metadata);
    assert!(forged.metadata.is_none());
    assert!(
        everruns_contracts::public_metadata::message(
            &json!({(KEY): {"version":1,"content":"private"}})
        )
        .as_object()
        .unwrap()
        .is_empty()
    );
}

#[test]
fn an_empty_inventory_is_disclosed_after_a_history_cut() {
    let mut history = vec![RuntimeMessage::user(
        "Summary: previously installed Skill A",
    )];
    let updates = prepare(&mut history, Some("No skills are currently available."));
    assert_eq!(updates.added(), 1);
    assert!(
        history
            .last()
            .unwrap()
            .text()
            .unwrap()
            .contains("No skills are currently available.")
    );
}

// Scenario from https://github.com/vercel/eve/pull/6498: a replayable
// announcement must precede an unreplayable live-context tail.
#[test]
fn continuation_catalog_update_survives_live_facts_and_the_next_turn_as_the_same_wire_prefix() {
    use super::super::{context_plan, facts};
    use everruns_contracts::runtime::turn_agent::TurnAgent;

    for turn_scoped in [false, true] {
        let wire = |messages: &[RuntimeMessage]| {
            messages
                .iter()
                .map(|source| {
                    let mut message = crate::llm_conversions::llm_message_from_message(source);
                    facts::mark_turn_scoped(&mut message, source, turn_scoped);
                    format!("{message:?}")
                })
                .collect::<Vec<_>>()
        };
        let render = |at: chrono::DateTime<chrono::Utc>| Some(format!("<facts>{at}</facts>"));
        let agent = TurnAgent {
            conversation_context: Some("Project instructions remain reference data".into()),
            conversation_context_update: Some("Current skill/tool catalog B".into()),
            ..Default::default()
        };
        let summary =
            context_plan::summary_message("Historical model notes; verify original evidence.");
        let mut raw = vec![
            summary.clone(),
            RuntimeMessage::user("task"),
            RuntimeMessage::assistant("partial"),
        ];
        let (mut first, volatile) = facts::interleave_facts(raw.clone(), render, turn_scoped);
        assert_eq!(volatile, 1);
        let update = apply(&mut first, &agent, volatile);
        let announcement = first
            .iter()
            .position(|m| {
                m.text()
                    .is_some_and(|t| t.contains("<capability-context-update>"))
            })
            .unwrap();
        assert!(
            announcement < first.len() - volatile,
            "a persistent update must precede the live-only facts"
        );
        assert_eq!(
            first[1].text(),
            summary.text(),
            "do not move ephemeral data across the compaction checkpoint"
        );
        let cached = wire(&first[..first.len() - volatile]);

        // The answer is persisted and reloaded; no worker-local update state.
        let mut tool_call = answer(&update);
        tool_call
            .content
            .push(crate::message::ContentPart::tool_call(
                "read",
                "read_file",
                json!({"path":"notes.md"}),
            ));
        raw.push(serde_json::from_value(serde_json::to_value(tool_call).unwrap()).unwrap());
        raw.push(RuntimeMessage::tool_result(
            "read",
            Some(json!("recorded result")),
            None,
        ));
        let (mut step, step_volatile) = facts::interleave_facts(raw.clone(), render, turn_scoped);
        assert_eq!(apply(&mut step, &agent, step_volatile).added(), 0);
        assert_eq!(step_volatile, 0);
        assert_eq!(wire(&step[..cached.len()]), cached);

        raw.push(RuntimeMessage::assistant("finished tool step"));
        raw.push(RuntimeMessage::user("next turn"));
        let (mut next, next_volatile) = facts::interleave_facts(raw, render, turn_scoped);
        let replay = apply(&mut next, &agent, next_volatile);
        assert_eq!(next_volatile, 0);
        assert_eq!(replay.added(), 0);
        assert_eq!(wire(&next[..cached.len()]), cached);
    }
}
