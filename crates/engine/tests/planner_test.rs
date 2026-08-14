//! Pure turn-planner unit tests (EVE-840).
//!
//! These drive the engine as table tests over values — no adapter, store,
//! provider, or clock. They cover the branch outcomes the runtime host relies
//! on (unknown tool, parallel tool batch, completion, failure, cancellation,
//! max-iteration, act pause/continue) plus a serialize → deserialize → plan
//! round-trip proving determinism.

use chrono::{DateTime, Utc};
use everruns_core::atoms::ReasonResult;
use everruns_core::events::TokenUsage;
use everruns_core::turn::TurnStopReason;
use everruns_engine::{
    ActOutcome, ActSchedulingFacts, TurnLifecycleEffect, TurnPlan, TurnState, plan_after_act,
    plan_after_reason, reason_schedules_act,
};
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{HarnessId, MessageId, SessionId, TurnId, WorkspaceId};
use serde_json::json;
use uuid::Uuid;

fn fixed_now() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
}

fn turn_state() -> TurnState {
    TurnState {
        org_id: 1,
        session_id: SessionId::from_uuid(Uuid::now_v7()),
        harness_id: HarnessId::from_uuid(Uuid::now_v7()),
        agent_id: None,
        input_message_id: MessageId::from_uuid(Uuid::now_v7()),
        turn_id: Some(TurnId::from_uuid(Uuid::now_v7())),
        previous_response_id: None,
        iteration: 1,
        request_id: None,
        started_at: None,
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    }
}

fn reason_result() -> ReasonResult {
    ReasonResult {
        success: true,
        text: String::new(),
        tool_calls: vec![],
        has_tool_calls: false,
        tool_definitions: vec![],
        max_iterations: 8,
        error: None,
        user_facing_error: None,
        error_disclosure: None,
        usage: None,
        output_message_id: None,
        time_to_first_token_ms: None,
        response_id: None,
        finish_reason: Some("stop".into()),
        locale: None,
        network_access: None,
        parallel_tool_calls: None,
    }
}

fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: json!({}),
    }
}

/// A reason outcome that requests a tool call (even one the executor may not
/// know) schedules an act phase carrying the calls, with the host-resolved
/// blueprint + workspace and no lifecycle effects.
#[test]
fn reason_with_tool_call_schedules_act() {
    let state = turn_state();
    let workspace_id = WorkspaceId::from_uuid(Uuid::now_v7());
    let result = ReasonResult {
        text: "calling an unknown tool".into(),
        tool_calls: vec![tool_call("call_1", "mystery_tool")],
        has_tool_calls: true,
        response_id: Some("resp_1".into()),
        finish_reason: Some("tool_calls".into()),
        parallel_tool_calls: Some(true),
        ..reason_result()
    };

    assert!(reason_schedules_act(&state, &result));

    let (plan, effects) = plan_after_reason(
        &state,
        result,
        0,
        fixed_now(),
        Some(ActSchedulingFacts {
            blueprint_id: Some("blueprint.private".into()),
            workspace_id: Some(workspace_id),
        }),
    );
    assert!(effects.is_empty());
    match plan {
        TurnPlan::ScheduleAct(act) => {
            assert_eq!(act.input.tool_calls.len(), 1);
            assert_eq!(act.input.tool_calls[0].name, "mystery_tool");
            assert_eq!(act.input.blueprint_id.as_deref(), Some("blueprint.private"));
            assert_eq!(act.input.context.workspace_id, Some(workspace_id));
            assert_eq!(act.previous_response_id.as_deref(), Some("resp_1"));
            assert_eq!(act.iteration, 1);
            assert_eq!(act.input.parallel_tool_calls, Some(true));
        }
        other => panic!("expected ScheduleAct, got {other:?}"),
    }
}

/// A parallel batch of tool calls threads through into one act phase intact.
#[test]
fn reason_with_parallel_tool_batch_schedules_single_act() {
    let state = turn_state();
    let result = ReasonResult {
        text: "parallel work".into(),
        tool_calls: vec![
            tool_call("call_a", "search"),
            tool_call("call_b", "read"),
            tool_call("call_c", "write"),
        ],
        has_tool_calls: true,
        finish_reason: Some("tool_calls".into()),
        ..reason_result()
    };

    let (plan, effects) = plan_after_reason(&state, result, 0, fixed_now(), None);
    assert!(effects.is_empty());
    match plan {
        TurnPlan::ScheduleAct(act) => {
            assert_eq!(act.input.tool_calls.len(), 3);
            // Facts absent -> no blueprint/workspace attached.
            assert_eq!(act.input.blueprint_id, None);
            assert_eq!(act.input.context.workspace_id, None);
        }
        other => panic!("expected ScheduleAct, got {other:?}"),
    }
}

/// A successful reason with no tool calls and no pending steering completes the
/// turn and returns the completion lifecycle effects in order.
#[test]
fn reason_success_completes_turn_with_effects() {
    let mut state = turn_state();
    state.started_at = Some(fixed_now() - chrono::Duration::milliseconds(1_234));
    let final_message_id = MessageId::from_uuid(Uuid::now_v7());
    let result = ReasonResult {
        text: "the final answer".into(),
        usage: Some(TokenUsage::new(20, 8)),
        output_message_id: Some(final_message_id),
        time_to_first_token_ms: Some(50),
        finish_reason: Some("stop".into()),
        ..reason_result()
    };

    let (plan, effects) = plan_after_reason(&state, result, 0, fixed_now(), None);
    assert!(matches!(
        plan,
        TurnPlan::Complete {
            stop_reason: TurnStopReason::EndTurn,
            error: None,
        }
    ));
    assert_eq!(effects.len(), 3);
    match &effects[0] {
        TurnLifecycleEffect::TurnCompleted {
            input_message_id,
            data,
        } => {
            assert_eq!(*input_message_id, state.input_message_id);
            assert_eq!(data.final_message_id, Some(final_message_id));
            assert_eq!(
                data.final_answer_preview.as_deref(),
                Some("the final answer")
            );
            assert_eq!(data.llm_call_count, Some(1));
            assert_eq!(data.duration_ms, Some(1_234));
            let usage = data.usage.as_ref().expect("usage aggregated");
            assert_eq!(usage.input_tokens, 20);
            assert_eq!(usage.output_tokens, 8);
        }
        other => panic!("expected TurnCompleted first, got {other:?}"),
    }
    assert!(matches!(
        effects[1],
        TurnLifecycleEffect::SessionIdled { .. }
    ));
    assert!(matches!(
        effects[2],
        TurnLifecycleEffect::FireTurnEndHooks { success: true, .. }
    ));
}

/// A failed reason completes with an error stop reason and returns the
/// turn-failed + turn-end-hook effects (no completion/idle pair).
#[test]
fn reason_failure_completes_with_failure_effect() {
    let state = turn_state();
    let result = ReasonResult {
        success: false,
        text: "budget exhausted".into(),
        has_tool_calls: false,
        error: Some("Budget exhausted".into()),
        finish_reason: None,
        ..reason_result()
    };

    let (plan, effects) = plan_after_reason(&state, result, 0, fixed_now(), None);
    match plan {
        TurnPlan::Complete { stop_reason, error } => {
            assert_eq!(stop_reason, TurnStopReason::Error);
            assert_eq!(error.as_deref(), Some("Budget exhausted"));
        }
        other => panic!("expected Complete, got {other:?}"),
    }
    assert_eq!(effects.len(), 2);
    match &effects[0] {
        TurnLifecycleEffect::TurnFailedWithDisclosure { text, .. } => {
            assert_eq!(text, "budget exhausted");
        }
        other => panic!("expected TurnFailedWithDisclosure, got {other:?}"),
    }
    assert!(matches!(
        effects[1],
        TurnLifecycleEffect::FireTurnEndHooks { success: false, .. }
    ));
}

/// A blocked act (host-side cancellation / dependency block) ends the turn with
/// no effects.
#[test]
fn act_blocked_completes_end_turn() {
    let state = turn_state();
    let (plan, effects) = plan_after_act(
        &state,
        ActOutcome {
            blocked: true,
            waiting_for_tool_results: false,
        },
        false,
    );
    assert!(effects.is_empty());
    assert!(matches!(
        plan,
        TurnPlan::Complete {
            stop_reason: TurnStopReason::EndTurn,
            error: None,
        }
    ));
}

/// When the iteration budget is already spent, a reason that still wants tools
/// surfaces `MaxTurnRequests` instead of scheduling another act.
#[test]
fn reason_at_max_iterations_surfaces_max_turn_requests() {
    let state = turn_state(); // iteration = 1
    let result = ReasonResult {
        text: "still calling tools".into(),
        tool_calls: vec![tool_call("call_x", "multiply")],
        has_tool_calls: true,
        max_iterations: 1, // reached
        finish_reason: Some("tool_calls".into()),
        ..reason_result()
    };

    assert!(!reason_schedules_act(&state, &result));
    let (plan, effects) = plan_after_reason(&state, result, 0, fixed_now(), None);
    assert!(matches!(
        plan,
        TurnPlan::Complete {
            stop_reason: TurnStopReason::MaxTurnRequests,
            error: None,
        }
    ));
    // Terminal: completion + idle + turn-end-hook effects.
    assert_eq!(effects.len(), 3);
}

/// An act awaiting tool results pauses only when the setup_connection hint is
/// enabled, emitting the waiting effect and bumping the iteration.
#[test]
fn act_waiting_pauses_when_hint_enabled() {
    let state = turn_state();
    let (plan, effects) = plan_after_act(
        &state,
        ActOutcome {
            blocked: false,
            waiting_for_tool_results: true,
        },
        true,
    );
    match plan {
        TurnPlan::WaitForToolResults { resume } => {
            assert_eq!(resume.iteration, 2);
            assert_eq!(resume.turn_id, state.turn_id);
        }
        other => panic!("expected WaitForToolResults, got {other:?}"),
    }
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        effects[0],
        TurnLifecycleEffect::WaitingForToolResults
    ));
}

/// Without the hint, an act awaiting tool results continues to reason instead.
#[test]
fn act_waiting_continues_when_hint_absent() {
    let state = turn_state();
    let (plan, effects) = plan_after_act(
        &state,
        ActOutcome {
            blocked: false,
            waiting_for_tool_results: true,
        },
        false,
    );
    assert!(effects.is_empty());
    match plan {
        TurnPlan::ScheduleReason(next) => assert_eq!(next.iteration, 2),
        other => panic!("expected ScheduleReason, got {other:?}"),
    }
}

/// Serializing a `TurnState`, deserializing it, and planning from both must
/// yield the same next plan for the same input — the durable-replay property in
/// miniature.
#[test]
fn serialize_deserialize_plan_round_trip_is_equal() {
    let mut state = turn_state();
    state.started_at = Some(fixed_now() - chrono::Duration::milliseconds(500));
    state.cumulative_usage = Some(TokenUsage::new(3, 1));
    state.tool_call_count = 2;
    state.llm_call_count = 1;

    let serialized = serde_json::to_string(&state).expect("serialize");
    let restored: TurnState = serde_json::from_str(&serialized).expect("deserialize");

    let now = fixed_now();
    let (plan_a, effects_a) = plan_after_reason(&state, reason_result(), 0, now, None);
    let (plan_b, effects_b) = plan_after_reason(&restored, reason_result(), 0, now, None);

    // Both terminal completions with identical stop reason + summarized fields.
    let (sr_a, err_a) = match plan_a {
        TurnPlan::Complete { stop_reason, error } => (stop_reason, error),
        other => panic!("expected Complete, got {other:?}"),
    };
    let (sr_b, err_b) = match plan_b {
        TurnPlan::Complete { stop_reason, error } => (stop_reason, error),
        other => panic!("expected Complete, got {other:?}"),
    };
    assert_eq!(sr_a, sr_b);
    assert_eq!(err_a, err_b);

    let completed = |effects: &[TurnLifecycleEffect]| -> everruns_core::events::TurnCompletedData {
        match &effects[0] {
            TurnLifecycleEffect::TurnCompleted { data, .. } => data.clone(),
            other => panic!("expected TurnCompleted, got {other:?}"),
        }
    };
    let data_a = completed(&effects_a);
    let data_b = completed(&effects_b);
    assert_eq!(data_a.duration_ms, data_b.duration_ms);
    assert_eq!(data_a.tool_call_count, data_b.tool_call_count);
    assert_eq!(data_a.llm_call_count, data_b.llm_call_count);
    assert_eq!(
        serde_json::to_value(&data_a.usage).unwrap(),
        serde_json::to_value(&data_b.usage).unwrap()
    );
}
