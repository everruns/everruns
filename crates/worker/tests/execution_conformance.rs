//! Cross-driver conformance for the shared engine execution.
//!
//! Every scenario advances the immediate driver continuously and restores the
//! durable driver from a serialized checkpoint between *every* phase. Plans,
//! lifecycle-effect order, and engine state must remain byte-for-byte
//! equivalent. Phase-local event emission is shared engine code; these tests
//! cover the driver-specific transition and lifecycle-event boundary.

use chrono::{TimeZone, Utc};
use everruns_durable::DurableExecution;
use everruns_engine::{
    ActOutcome, ActivityOutcome, Execution, HostFacts, ReasonResult, TurnLifecycleEffect, TurnPlan,
    TurnState,
};
use everruns_host::InProcessExecution;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use serde_json::json;

fn initial_state() -> TurnState {
    TurnState {
        org_id: 1,
        session_id: SessionId::new(),
        harness_id: HarnessId::new(),
        agent_id: None,
        input_message_id: MessageId::new(),
        turn_id: Some(TurnId::new()),
        previous_response_id: None,
        iteration: 1,
        request_id: Some("request-1".to_string()),
        started_at: Some(Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap()),
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    }
}

fn reason(text: &str) -> ReasonResult {
    ReasonResult {
        success: true,
        text: text.to_string(),
        max_iterations: 10,
        finish_reason: Some("stop".to_string()),
        ..ReasonResult::default()
    }
}

fn tool_reason(iteration_limit: usize) -> ReasonResult {
    ReasonResult {
        success: true,
        text: "using a tool".to_string(),
        tool_calls: vec![ToolCall {
            id: "call-1".to_string(),
            name: "lookup".to_string(),
            arguments: json!({"query": "everruns"}),
        }],
        has_tool_calls: true,
        response_id: Some("response-1".to_string()),
        finish_reason: Some("tool_calls".to_string()),
        max_iterations: iteration_limit,
        ..ReasonResult::default()
    }
}

fn effect_sequence(effects: &[TurnLifecycleEffect]) -> Vec<&'static str> {
    effects
        .iter()
        .map(|effect| match effect {
            TurnLifecycleEffect::TurnCompleted { .. } => "turn.completed",
            TurnLifecycleEffect::SessionIdled { .. } => "session.idled",
            TurnLifecycleEffect::TurnFailedWithDisclosure { .. } => "turn.failed",
            TurnLifecycleEffect::FireTurnEndHooks { .. } => "hook.turn_end",
            TurnLifecycleEffect::WaitingForToolResults => "session.waiting_for_tool_results",
        })
        .collect()
}

fn plan_kind(plan: &TurnPlan) -> &'static str {
    match plan {
        TurnPlan::ScheduleReason(_) => "reason",
        TurnPlan::ScheduleAct(_) => "act",
        TurnPlan::Complete { .. } => "complete",
        TurnPlan::WaitForToolResults { .. } => "wait",
    }
}

fn advance_both(
    immediate: &mut InProcessExecution,
    durable: &mut DurableExecution,
    immediate_outcome: ActivityOutcome,
    durable_outcome: ActivityOutcome,
    pending: usize,
    facts: HostFacts,
) -> (String, Vec<&'static str>) {
    let now = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();
    let immediate_transition = immediate.advance(immediate_outcome, pending, now, facts.clone());
    let durable_transition = durable.advance(durable_outcome, pending, now, facts);

    assert_eq!(
        normalized_debug(&immediate_transition.plan),
        normalized_debug(&durable_transition.plan),
        "drivers selected different behavior"
    );
    assert_eq!(
        normalized_debug(&immediate_transition.effects),
        normalized_debug(&durable_transition.effects),
        "drivers selected different lifecycle event sequence"
    );
    assert_eq!(
        serde_json::to_value(immediate.state()).unwrap(),
        serde_json::to_value(durable.state()).unwrap(),
        "drivers retained different engine state"
    );

    // The durable contract is stronger than cloning: discard the live value
    // and restore the exact serialized checkpoint after every phase.
    let encoded = serde_json::to_vec(durable).expect("serialize durable execution");
    *durable = serde_json::from_slice(&encoded).expect("restore durable execution");

    (
        plan_kind(&immediate_transition.plan).to_string(),
        effect_sequence(&immediate_transition.effects),
    )
}

fn normalized_debug(value: &impl std::fmt::Debug) -> String {
    let mut rendered = format!("{value:?}");
    const PREFIX: &str = "ExecIdMarker(exec_";
    while let Some(start) = rendered.find(PREFIX) {
        let value_start = start + "ExecIdMarker(".len();
        let end = rendered[value_start..]
            .find(')')
            .map(|offset| value_start + offset)
            .expect("execution id debug value closes");
        rendered.replace_range(value_start..end, "<driver-local>");
    }
    rendered
}

fn drivers() -> (InProcessExecution, DurableExecution) {
    let state = initial_state();
    (
        InProcessExecution::new(state.clone()),
        DurableExecution::new(state),
    )
}

#[test]
fn plain_completion_has_identical_behavior_and_event_order() {
    let (mut immediate, mut durable) = drivers();
    let turn_id = TurnId::new();
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::ProcessInput {
                turn_id: Some(turn_id)
            },
            ActivityOutcome::ProcessInput {
                turn_id: Some(turn_id)
            },
            0,
            HostFacts::default(),
        ),
        ("reason".into(), vec![])
    );
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(reason("done"))),
            ActivityOutcome::Reason(Box::new(reason("done"))),
            0,
            HostFacts::default(),
        ),
        (
            "complete".into(),
            vec!["turn.completed", "session.idled", "hook.turn_end"]
        )
    );
}

#[test]
fn tool_round_trip_has_identical_behavior_and_state() {
    let (mut immediate, mut durable) = drivers();
    let turn_id = TurnId::new();
    advance_both(
        &mut immediate,
        &mut durable,
        ActivityOutcome::ProcessInput {
            turn_id: Some(turn_id),
        },
        ActivityOutcome::ProcessInput {
            turn_id: Some(turn_id),
        },
        0,
        HostFacts::default(),
    );
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(tool_reason(10))),
            ActivityOutcome::Reason(Box::new(tool_reason(10))),
            0,
            HostFacts::default(),
        ),
        ("act".into(), vec![])
    );
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Act(ActOutcome::default()),
            ActivityOutcome::Act(ActOutcome::default()),
            0,
            HostFacts::default(),
        ),
        ("reason".into(), vec![])
    );
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(reason("tool result used"))),
            ActivityOutcome::Reason(Box::new(reason("tool result used"))),
            0,
            HostFacts::default(),
        )
        .1,
        vec!["turn.completed", "session.idled", "hook.turn_end"]
    );
    assert_eq!(immediate.state().tool_call_count, 1);
    assert_eq!(immediate.state().llm_call_count, 2);
}

#[test]
fn steering_failure_limit_block_and_wait_branches_are_equivalent() {
    // Steering keeps the same turn in Reason.
    let (mut immediate, mut durable) = drivers();
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(reason("not final"))),
            ActivityOutcome::Reason(Box::new(reason("not final"))),
            2,
            HostFacts::default(),
        )
        .0,
        "reason"
    );

    // Provider failure preserves turn.failed -> hook ordering.
    let (mut immediate, mut durable) = drivers();
    let failed = || ReasonResult {
        success: false,
        text: "request failed".to_string(),
        error: Some("provider unavailable".to_string()),
        finish_reason: Some("error".to_string()),
        max_iterations: 10,
        ..ReasonResult::default()
    };
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(failed())),
            ActivityOutcome::Reason(Box::new(failed())),
            0,
            HostFacts::default(),
        )
        .1,
        vec!["turn.failed", "hook.turn_end"]
    );

    // Max-turn ceiling rejects a tool batch that cannot be run.
    let state = TurnState {
        iteration: 2,
        ..initial_state()
    };
    let mut immediate = InProcessExecution::new(state.clone());
    let mut durable = DurableExecution::new(state);
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Reason(Box::new(tool_reason(2))),
            ActivityOutcome::Reason(Box::new(tool_reason(2))),
            0,
            HostFacts::default(),
        )
        .0,
        "complete"
    );

    // Blocked acts terminate, while client-side tool waits are explicit and
    // retain a resumable checkpoint only when the host enables the hint.
    let (mut immediate, mut durable) = drivers();
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Act(ActOutcome {
                blocked: true,
                waiting_for_tool_results: false,
            }),
            ActivityOutcome::Act(ActOutcome {
                blocked: true,
                waiting_for_tool_results: false,
            }),
            0,
            HostFacts::default(),
        )
        .0,
        "complete"
    );

    let (mut immediate, mut durable) = drivers();
    assert_eq!(
        advance_both(
            &mut immediate,
            &mut durable,
            ActivityOutcome::Act(ActOutcome {
                blocked: false,
                waiting_for_tool_results: true,
            }),
            ActivityOutcome::Act(ActOutcome {
                blocked: false,
                waiting_for_tool_results: true,
            }),
            0,
            HostFacts {
                setup_connection_hint_enabled: true,
                ..HostFacts::default()
            },
        ),
        ("wait".into(), vec!["session.waiting_for_tool_results"])
    );
}
