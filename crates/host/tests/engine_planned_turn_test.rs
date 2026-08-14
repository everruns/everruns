//! Parity coverage for the engine-planned in-process turn loop (EVE-842).
//!
//! `InProcessRuntime::run_turn` no longer decides reason-vs-act-vs-complete on
//! its own — every step comes from `everruns-engine`'s planner and the loop only
//! executes the named host operation. These tests pin the observable contract
//! that migration had to preserve: the `TurnResult` summary values and the
//! turn-scoped event sequence, across single/parallel tools, tool errors,
//! provider failure, and the max-iteration ceiling.
//!
//! The last test covers the durable property the engine buys us: the planner's
//! carried state survives a serialize/reconstruct between every step, so a host
//! that persists it between activities plans the identical turn.

use everruns_core::{AgentDefinition, CapabilityRegistry, ExecutionSession};
use everruns_engine::{
    ActOutcome, TurnPlan, TurnState, plan_after_act, plan_after_process_input, plan_after_reason,
};
use everruns_host::HostComposition;
use everruns_host::{
    AgentBuilder, HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder, SessionBuilder,
    TurnStopReason,
};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimError, SimToolCall, SimTurn};
use serde_json::json;

fn math_platform() -> HostComposition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    HostComposition::new(capabilities, DriverRegistry::new())
}

fn harness(harness_id: everruns_provider::typed_id::HarnessId) -> everruns_host::SeededHarness {
    HarnessBuilder::new("math", "You are a math assistant.")
        .id(harness_id)
        .capability("test_math")
        .build()
}

fn agent(agent_id: everruns_provider::typed_id::AgentId, max_iterations: usize) -> AgentDefinition {
    AgentBuilder::new("math-agent", "Use tools when needed.")
        .id(agent_id)
        .display_name("Math Agent")
        .max_iterations(max_iterations)
        .build()
}

fn session(
    session_id: everruns_provider::typed_id::SessionId,
    harness_id: everruns_provider::typed_id::HarnessId,
    agent_id: everruns_provider::typed_id::AgentId,
) -> ExecutionSession {
    SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .title("Engine Planned ExecutionSession")
        .build()
}

fn add(id: &str, a: i64, b: i64) -> SimToolCall {
    SimToolCall {
        name: "add".to_string(),
        arguments: json!({ "a": a, "b": b }),
        id: Some(id.to_string()),
    }
}

/// Build a runtime whose LLM turns follow `script`, seeded deterministically so
/// concurrent tests never share ids.
async fn runtime_running(
    seed: u128,
    max_iterations: usize,
    script: Vec<SimTurn>,
) -> (InProcessRuntime, everruns_provider::typed_id::SessionId) {
    let harness_id = everruns_provider::typed_id::HarnessId::from_seed(seed);
    let agent_id = everruns_provider::typed_id::AgentId::from_seed(seed);
    let session_id = everruns_provider::typed_id::SessionId::from_seed(seed);

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(math_platform())
        .harness(harness(harness_id))
        .agent(agent(agent_id, max_iterations))
        .session(session(session_id, harness_id, agent_id))
        .llm_sim(LlmSimConfig::scripted(script))
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .build()
        .await
        .expect("runtime builds");

    (runtime, session_id)
}

async fn event_types(runtime: &InProcessRuntime) -> Vec<String> {
    runtime
        .events()
        .await
        .expect("events")
        .into_iter()
        .map(|event| event.data.event_type().to_string())
        .collect()
}

/// Turn-scoped lifecycle events, with the per-tool / per-message noise dropped
/// so the assertion pins the sequence the planner is responsible for.
fn lifecycle_sequence(event_types: &[String]) -> Vec<&str> {
    event_types
        .iter()
        .map(String::as_str)
        .filter(|event_type| {
            matches!(
                *event_type,
                "input.message"
                    | "session.activated"
                    | "turn.started"
                    | "turn.completed"
                    | "turn.failed"
                    | "session.idled"
            )
        })
        .collect()
}

#[tokio::test]
async fn single_tool_turn_reports_two_iterations_and_one_tool_call() {
    let (runtime, session_id) = runtime_running(
        901,
        8,
        vec![
            SimTurn::ToolCalls(vec![add("call_1", 2, 2)]),
            SimTurn::Assistant("The answer is 4.".to_string()),
        ],
    )
    .await;

    let result = runtime
        .run_text_turn(session_id, "What is 2 + 2?")
        .await
        .expect("turn runs");

    assert!(result.success, "expected success, got {result:?}");
    assert_eq!(result.response, "The answer is 4.");
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(result.error, None);
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);

    let types = event_types(&runtime).await;
    assert_eq!(
        lifecycle_sequence(&types),
        vec![
            "input.message",
            "session.activated",
            "turn.started",
            "turn.completed",
            "session.idled",
        ],
    );
    assert!(
        types
            .iter()
            .any(|event_type| event_type == "tool.completed"),
        "expected the planned act to run a tool: {types:?}"
    );
}

#[tokio::test]
async fn parallel_tool_batch_runs_as_one_planned_act() {
    let (runtime, session_id) = runtime_running(
        902,
        8,
        vec![
            SimTurn::ToolCalls(vec![add("call_1", 1, 1), add("call_2", 3, 4)]),
            SimTurn::Assistant("2 and 7.".to_string()),
        ],
    )
    .await;

    let result = runtime
        .run_text_turn(session_id, "Add these")
        .await
        .expect("turn runs");

    assert!(result.success);
    // Both calls land in a single act plan, so the turn still costs two reasons.
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_count, 2);

    let types = event_types(&runtime).await;
    let tool_completions = types
        .iter()
        .filter(|event_type| *event_type == "tool.completed")
        .count();
    assert_eq!(tool_completions, 2, "{types:?}");
}

#[tokio::test]
async fn failing_tool_does_not_fail_the_turn() {
    // `divide` by zero is a tool-level error: the act reports it as a result and
    // the engine keeps planning, so the turn completes normally.
    let (runtime, session_id) = runtime_running(
        903,
        8,
        vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "divide".to_string(),
                arguments: json!({ "a": 1, "b": 0 }),
                id: Some("call_1".to_string()),
            }]),
            SimTurn::Assistant("That division is undefined.".to_string()),
        ],
    )
    .await;

    let result = runtime
        .run_text_turn(session_id, "Divide 1 by 0")
        .await
        .expect("turn runs");

    assert!(result.success, "tool errors are results, not turn failures");
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);
    assert_eq!(
        lifecycle_sequence(&event_types(&runtime).await),
        vec![
            "input.message",
            "session.activated",
            "turn.started",
            "turn.completed",
            "session.idled",
        ],
    );
}

#[tokio::test]
async fn provider_failure_completes_the_turn_as_failed() {
    let (runtime, session_id) =
        runtime_running(904, 8, vec![SimTurn::Error(SimError::Authentication)]).await;

    let result = runtime
        .run_text_turn(session_id, "hello")
        .await
        .expect("turn runs");

    assert!(!result.success);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls_count, 0);
    assert_eq!(result.response, "");
    assert!(result.error.is_some(), "failure must carry the error text");
    assert_eq!(result.stop_reason, TurnStopReason::Error);

    assert_eq!(
        lifecycle_sequence(&event_types(&runtime).await),
        vec![
            "input.message",
            "session.activated",
            "turn.started",
            "turn.failed",
            "session.idled",
        ],
    );
}

#[tokio::test]
async fn tool_calls_at_the_iteration_ceiling_stop_with_max_turn_requests() {
    // Every scripted turn asks for another tool call, so the ceiling is what
    // ends the turn rather than the model.
    let (runtime, session_id) = runtime_running(
        905,
        2,
        vec![
            SimTurn::ToolCalls(vec![add("call_1", 1, 1)]),
            SimTurn::ToolCalls(vec![add("call_2", 2, 2)]),
            SimTurn::ToolCalls(vec![add("call_3", 3, 3)]),
        ],
    )
    .await;

    let result = runtime
        .run_text_turn(session_id, "keep going")
        .await
        .expect("turn runs");

    assert!(result.success);
    assert_eq!(result.iterations, 2);
    // The second reason's batch is never acted on — the ceiling stops it first.
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(result.stop_reason, TurnStopReason::MaxTurnRequests);
    assert_eq!(
        lifecycle_sequence(&event_types(&runtime).await),
        vec![
            "input.message",
            "session.activated",
            "turn.started",
            "turn.completed",
            "session.idled",
        ],
    );
}

#[tokio::test]
async fn cancelling_a_turn_leaves_the_runtime_usable() {
    // The facade cancels by dropping the `run_turn` future. Nothing in the
    // engine-planned loop holds a lock across an await point, so a later turn on
    // the same session still runs to completion.
    let (runtime, session_id) = runtime_running(
        906,
        8,
        vec![
            SimTurn::ToolCalls(vec![add("call_1", 2, 2)]),
            SimTurn::Assistant("4".to_string()),
            SimTurn::Assistant("still here".to_string()),
        ],
    )
    .await;

    let cancelled = tokio::time::timeout(
        std::time::Duration::from_nanos(1),
        runtime.run_text_turn(session_id, "start something"),
    )
    .await;
    assert!(cancelled.is_err(), "expected the turn future to be dropped");

    let result = runtime
        .run_text_turn(session_id, "and again")
        .await
        .expect("a later turn still runs");
    assert!(result.success, "{result:?}");
}

/// Restart-between-steps: serialize the planner's carried state after every
/// plan, reconstruct it, and continue from the reconstructed copy. A host that
/// persists state between activities must plan the identical turn.
#[test]
fn planner_state_survives_a_restart_between_every_step() {
    fn round_trip(state: &TurnState) -> TurnState {
        let encoded = serde_json::to_value(state).expect("state serializes");
        serde_json::from_value(encoded).expect("state deserializes")
    }

    let session_id = everruns_provider::typed_id::SessionId::from_seed(907);
    let harness_id = everruns_provider::typed_id::HarnessId::from_seed(907);
    let input_message_id = everruns_provider::typed_id::MessageId::from_seed(907);
    let turn_id = everruns_provider::typed_id::TurnId::from_seed(907);
    let now = chrono::Utc::now();

    let initial = TurnState {
        org_id: 1,
        session_id,
        harness_id,
        agent_id: None,
        input_message_id,
        turn_id: None,
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
    };

    // process_input -> reason
    let TurnPlan::ScheduleReason(state) =
        plan_after_process_input(&round_trip(&initial), Some(turn_id), now)
    else {
        panic!("process_input must schedule a reason");
    };
    assert_eq!(state.turn_id, Some(turn_id));
    assert_eq!(state.iteration, 1);

    // reason (with tool calls) -> act
    let with_tools = everruns_engine::ReasonResult {
        success: true,
        has_tool_calls: true,
        max_iterations: 8,
        text: "calling a tool".to_string(),
        response_id: Some("resp_1".to_string()),
        tool_calls: vec![everruns_provider::tool_types::ToolCall {
            id: "call_1".to_string(),
            name: "add".to_string(),
            arguments: json!({ "a": 1, "b": 1 }),
        }],
        ..Default::default()
    };

    let (plan, effects) = plan_after_reason(
        &round_trip(&state),
        with_tools,
        0,
        now,
        Some(Default::default()),
    );
    assert!(
        effects.is_empty(),
        "a continuing turn emits no lifecycle effects"
    );
    let TurnPlan::ScheduleAct(act_plan) = plan else {
        panic!("a reason with tool calls must schedule an act");
    };

    // The host applies the plan's response id / iteration onto the resumed
    // state, exactly as the durable worker does when it dequeues the act.
    let mut resumed = round_trip(act_plan.resume_state.as_ref());
    resumed.previous_response_id = act_plan.previous_response_id.clone();
    resumed.iteration = act_plan.iteration;
    assert_eq!(resumed.previous_response_id.as_deref(), Some("resp_1"));
    assert_eq!(resumed.tool_call_count, 1);
    assert_eq!(resumed.llm_call_count, 1);

    // act -> reason
    let (plan, effects) = plan_after_act(&round_trip(&resumed), ActOutcome::default(), false);
    assert!(effects.is_empty());
    let TurnPlan::ScheduleReason(state) = plan else {
        panic!("a completed act must schedule the next reason");
    };
    assert_eq!(state.iteration, 2);
    assert_eq!(state.previous_response_id.as_deref(), Some("resp_1"));

    // reason (no tool calls) -> complete
    let final_reason = everruns_engine::ReasonResult {
        success: true,
        max_iterations: 8,
        text: "done".to_string(),
        ..Default::default()
    };

    let (plan, effects) = plan_after_reason(&round_trip(&state), final_reason, 0, now, None);
    let TurnPlan::Complete { stop_reason, error } = plan else {
        panic!("a reason with no tool calls must complete the turn");
    };
    assert_eq!(stop_reason, TurnStopReason::EndTurn);
    assert_eq!(error, None);
    assert_eq!(
        effects.len(),
        3,
        "turn.completed + session.idled + turn_end hooks"
    );
}
