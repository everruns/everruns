use chrono::{TimeZone, Utc};
use everruns_durable::DurableExecution;
use everruns_engine::{ActivityOutcome, Execution, HostFacts, ReasonResult, TurnPlan, TurnState};
use everruns_host::InProcessExecution;
use everruns_provider::typed_id::{HarnessId, MessageId, SessionId, TurnId};

fn initial_state() -> TurnState {
    TurnState {
        org_id: 1,
        session_id: SessionId::new(),
        harness_id: HarnessId::new(),
        agent_id: None,
        input_message_id: MessageId::new(),
        turn_id: None,
        previous_response_id: None,
        iteration: 1,
        request_id: Some("request-1".to_string()),
        started_at: None,
        cumulative_usage: None,
        tool_call_count: 0,
        llm_call_count: 0,
        time_to_first_token_ms: None,
        final_message_id: None,
        final_answer_preview: None,
    }
}

#[test]
fn in_process_and_durable_executions_share_state_transitions() {
    let state = initial_state();
    let mut in_process = InProcessExecution::new(state.clone());
    let mut durable = DurableExecution::new(state);
    let turn_id = TurnId::new();
    let now = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();

    let immediate_transition = in_process.advance(
        ActivityOutcome::ProcessInput {
            turn_id: Some(turn_id),
        },
        0,
        now,
        HostFacts::default(),
    );
    let durable_transition = durable.advance(
        ActivityOutcome::ProcessInput {
            turn_id: Some(turn_id),
        },
        0,
        now,
        HostFacts::default(),
    );

    assert!(matches!(
        immediate_transition.plan,
        TurnPlan::ScheduleReason(_)
    ));
    assert!(matches!(
        durable_transition.plan,
        TurnPlan::ScheduleReason(_)
    ));
    assert!(immediate_transition.effects.is_empty());
    assert!(durable_transition.effects.is_empty());
    assert_eq!(
        serde_json::to_value(in_process.state()).unwrap(),
        serde_json::to_value(durable.state()).unwrap()
    );

    let reason = ReasonResult {
        success: true,
        text: "done".to_string(),
        max_iterations: 10,
        ..ReasonResult::default()
    };
    let immediate_transition = in_process.advance(
        ActivityOutcome::Reason(Box::new(reason.clone())),
        0,
        now,
        HostFacts::default(),
    );
    let durable_transition = durable.advance(
        ActivityOutcome::Reason(Box::new(reason)),
        0,
        now,
        HostFacts::default(),
    );

    assert!(matches!(
        immediate_transition.plan,
        TurnPlan::Complete { .. }
    ));
    assert!(matches!(durable_transition.plan, TurnPlan::Complete { .. }));
    assert_eq!(
        immediate_transition.effects.len(),
        durable_transition.effects.len()
    );
    assert_eq!(
        serde_json::to_value(in_process.state()).unwrap(),
        serde_json::to_value(durable.state()).unwrap()
    );
    assert_eq!(in_process.state().llm_call_count, 1);
}
