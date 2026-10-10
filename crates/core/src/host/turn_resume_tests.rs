//! The log fold and the rerun rule of [`super`].

use crate::events::{OutputMessageCompletedData, TurnCancelledData, TurnStartedData};
use crate::message::RuntimeMessage;

use super::*;

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "share".into(),
        arguments: serde_json::json!({ "to": id }),
    }
}

/// A turn that asked for `calls`, of which `settled` finished, and that
/// was cancelled when `cancelled`.
fn turn_log(calls: &[&str], settled: &[&str], cancelled: bool) -> (TurnId, Vec<Event>) {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let context = EventContext::turn(turn_id, input_message_id);
    let mut events = vec![
        Event::new(
            session_id,
            context.clone(),
            TurnStartedData {
                turn_id,
                input_message_id,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
            },
        ),
        Event::new(
            session_id,
            context.clone(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant_with_tools(
                "",
                calls.iter().map(|id| call(id)).collect(),
            )),
        ),
    ];
    for id in settled {
        events.push(Event::new(
            session_id,
            context.clone(),
            ToolCompletedData::success(id.to_string(), "share".into(), Vec::new(), None),
        ));
    }
    if cancelled {
        events.push(Event::new(
            session_id,
            context,
            TurnCancelledData {
                turn_id,
                reason: None,
                usage: None,
            },
        ));
    }
    (turn_id, events)
}

fn fold(events: Vec<Event>) -> Option<(TurnId, Vec<ToolCall>)> {
    TurnLogFold::from_events(events)
        .filter(|turn| !turn.is_answered())
        .map(|turn| (turn.turn_id(), turn.unfinished_calls()))
}

#[test]
fn a_turn_without_an_end_reports_its_unfinished_calls() {
    let (turn_id, events) = turn_log(&["a", "b"], &["a"], false);
    assert_eq!(fold(events), Some((turn_id, vec![call("b")])));
}

#[test]
fn an_ended_turn_is_not_interrupted() {
    assert_eq!(fold(turn_log(&["a"], &[], true).1), None);
}

#[test]
fn a_settled_act_without_an_end_reasons_again() {
    let (turn_id, events) = turn_log(&["a"], &["a"], false);
    assert_eq!(fold(events), Some((turn_id, vec![])));
}

#[test]
fn an_answered_turn_without_an_end_is_not_resumed() {
    assert_eq!(fold(turn_log(&[], &[], false).1), None);
}

#[test]
fn only_the_last_turn_counts() {
    let (_, mut events) = turn_log(&["a"], &[], false);
    let (last, later) = turn_log(&["b"], &["b"], false);
    events.extend(later);
    assert_eq!(fold(events), Some((last, vec![])));
}

fn tool(policy: ToolPolicy, class: Option<SideEffectClass>) -> ToolDefinition {
    let mut hints = everruns_contracts::tool_types::ToolHints::default();
    if let Some(class) = class {
        hints = hints.with_side_effect_class(class);
    }
    ToolDefinition::Builtin(everruns_contracts::tool_types::BuiltinTool {
        name: "share".into(),
        display_name: None,
        description: String::new(),
        parameters: serde_json::json!({}),
        policy,
        category: None,
        deferrable: Default::default(),
        hints,
        full_parameters: None,
    })
}

#[test]
fn only_calls_safe_to_run_twice_rerun() {
    let share = call("a");
    let auto = tool(ToolPolicy::Auto, None);
    assert!(
        !reruns(&share, Some(&auto), None),
        "at most once by default"
    );
    assert!(!reruns(&share, None, None), "a tool that is gone");
    for class in [SideEffectClass::Pure, SideEffectClass::Idempotent] {
        assert!(reruns(
            &share,
            Some(&tool(ToolPolicy::Auto, Some(class))),
            None
        ));
    }
    for policy in [ToolPolicy::ClientSide, ToolPolicy::RequiresApproval] {
        assert!(reruns(&share, Some(&tool(policy, None)), None));
    }
    let ask = ToolCall {
        name: ASK_USER_TOOL_NAME.into(),
        ..share.clone()
    };
    assert!(reruns(&ask, Some(&auto), None));
    let gated = |call: &ToolCall| call.arguments["to"] == "a";
    assert!(reruns(&share, Some(&auto), Some(&gated)));
    assert!(!reruns(&call("b"), Some(&auto), Some(&gated)));
}

#[test]
fn a_tail_from_the_last_turn_started_folds_like_the_whole_log() {
    let (_, mut events) = turn_log(&["a"], &[], true);
    let (last, later) = turn_log(&["b", "c"], &["b"], false);
    let tail_start = events.len();
    events.extend(later);
    let tail = events[tail_start..].to_vec();
    assert_eq!(fold(tail), Some((last, vec![call("c")])));
    assert_eq!(fold(events.clone()), Some((last, vec![call("c")])));
    // A tail cut after its `turn.started` holds no open turn.
    assert_eq!(fold(events[tail_start + 1..].to_vec()), None);
}

#[test]
fn a_parked_turn_with_its_results_recorded_has_nothing_left_to_run() {
    // A client-side call's `tool.completed` arrives after the park; the
    // open turn then only moves to its next reason.
    let (turn_id, events) = turn_log(&["client"], &["client"], false);
    let turn = TurnLogFold::from_events(events).expect("open turn");
    assert_eq!(turn.turn_id(), turn_id);
    assert!(!turn.is_answered());
    assert!(turn.unfinished_calls().is_empty());
}
