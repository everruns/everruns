// Thin host I/O wrapper over the pure turn planner in `everruns-engine`.
//
// Decision (EVE-840, Sans-IO Turn State epic): the authoritative turn-planning
// brain lives in `everruns-engine` as pure, deterministic functions. This
// module is the runtime host's I/O shell around it: it resolves the same facts
// via the adapter *in exactly the same conditions as before* (fetch the session
// only when scheduling an act; read the setup_connection hint only when the act
// paused for tool results), supplies `Utc::now()`, calls the engine planner,
// then performs any returned `TurnLifecycleEffect`s via `RuntimeSessionLifecycle`
// (identical order to the pre-extraction code), and returns the plan. The
// runtime carries no second copy of the planning brain.

use crate::{RuntimeHostAdapter, RuntimeSessionLifecycle};
use chrono::Utc;
use everruns_core::Controls;
use everruns_engine::{
    ActOutcome, ActSchedulingFacts, ActivityOutcome, Execution, HostFacts, ReasonResult,
    TurnLifecycleEffect, TurnPlan, TurnState, reason_schedules_act,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{SessionId, TurnId};

/// Determine the next host step after an activity finishes.
///
/// The host provides:
/// - `completed_activity`: which phase just finished
/// - `state`: host-carried turn state
/// - `output`: serialized activity output
/// - `pending_user_message_count`: number of queued steering messages already consumed by the host
///
/// The engine owns the semantic decision; this wrapper owns the I/O around it.
/// Hosts translate the returned plan into their own queueing / persistence model.
///
/// Typical host mapping:
/// - `ScheduleReason` => enqueue or invoke a reason phase with the returned state
/// - `ScheduleAct` => enqueue or invoke an act phase with the returned payload
/// - `WaitForToolResults` => persist the resume state until external tool input arrives
/// - `Complete` => mark the host-owned workflow/session turn complete
pub async fn advance_host_execution<A: RuntimeHostAdapter, E: Execution>(
    adapter: &A,
    execution: &mut E,
    completed_activity: &str,
    output: &serde_json::Value,
    pending_user_message_count: usize,
) -> Result<TurnPlan> {
    let state = execution.state().clone();
    match completed_activity {
        "process_input" => {
            let turn_id: Option<TurnId> = output
                .get("turn_id")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse().ok());
            let transition = execution.advance(
                ActivityOutcome::ProcessInput { turn_id },
                pending_user_message_count,
                Utc::now(),
                HostFacts::default(),
            );
            perform_effects(adapter, state.org_id, state.session_id, transition.effects).await?;
            Ok(transition.plan)
        }
        "reason" => {
            let reason_result: ReasonResult = serde_json::from_value(output.clone())
                .map_err(|error| AgentLoopError::Internal(error.into()))?;

            let act_scheduling = resolve_act_scheduling(adapter, &state, &reason_result).await?;

            let transition = execution.advance(
                ActivityOutcome::Reason(Box::new(reason_result)),
                pending_user_message_count,
                Utc::now(),
                HostFacts {
                    act_scheduling,
                    ..HostFacts::default()
                },
            );
            perform_effects(adapter, state.org_id, state.session_id, transition.effects).await?;
            Ok(transition.plan)
        }
        "act" => {
            let outcome = ActOutcome {
                blocked: output
                    .get("blocked")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                waiting_for_tool_results: output
                    .get("waiting_for_tool_results")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                waiting_for_url_elicitation: output
                    .get("waiting_for_url_elicitation")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            };

            let hints = resolve_pause_hints(adapter, state.org_id, state.session_id, outcome).await;

            let transition = execution.advance(
                ActivityOutcome::Act(outcome),
                pending_user_message_count,
                Utc::now(),
                HostFacts {
                    setup_connection_hint_enabled: hints.setup_connection,
                    url_elicitation_hint_enabled: hints.url_elicitation,
                    ..HostFacts::default()
                },
            );
            perform_effects(adapter, state.org_id, state.session_id, transition.effects).await?;
            Ok(transition.plan)
        }
        other => Err(AgentLoopError::config(format!(
            "Unknown activity type completed: {other}"
        ))),
    }
}

/// Resolve the act-scheduling session facts, fetching the session only when the
/// reason outcome actually schedules an act — the exact condition the
/// pre-extraction planner fetched under.
///
/// Shared by immediate and durable hosts so both resolve the same I/O facts
/// under the same condition.
pub(crate) async fn resolve_act_scheduling<A: RuntimeHostAdapter>(
    adapter: &A,
    state: &TurnState,
    reason_result: &ReasonResult,
) -> Result<Option<ActSchedulingFacts>> {
    if !reason_schedules_act(state, reason_result) {
        return Ok(None);
    }
    let session = adapter
        .session_store(state.org_id)
        .get_session(state.session_id)
        .await?;
    Ok(Some(ActSchedulingFacts {
        blueprint_id: session.as_ref().and_then(|s| s.blueprint_id.clone()),
        // Attach the session's workspace so tool file I/O addresses the
        // (possibly shared) workspace, not the session's own keyspace.
        workspace_id: session.as_ref().map(|s| s.workspace_id),
    }))
}

/// The pause hints a session declares: which kinds of pause its client can
/// actually answer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PauseHints {
    pub setup_connection: bool,
    pub url_elicitation: bool,
}

/// Resolve the pause hints only when the act actually paused for tool results,
/// so the session fetch happens under exactly the original condition.
pub(crate) async fn resolve_pause_hints<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    outcome: ActOutcome,
) -> PauseHints {
    if outcome.blocked || !outcome.waiting_for_tool_results {
        return PauseHints::default();
    }
    match adapter.session_store(org_id).get_session(session_id).await {
        Ok(Some(session)) => {
            let hints = Controls::resolve_hints(session.hints.as_ref(), None);
            let flag = |name: &str| {
                hints
                    .get(name)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            };
            PauseHints {
                setup_connection: flag("setup_connection"),
                url_elicitation: flag("url_elicitation"),
            }
        }
        _ => PauseHints::default(),
    }
}

/// Perform the engine-returned lifecycle effects, in list order, via
/// `RuntimeSessionLifecycle`. The engine never emits — it returns these — so
/// this is the single place the runtime host translates a planning decision
/// into event/status/hook I/O, preserving the exact stream and ordering.
pub(crate) async fn perform_effects<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    effects: Vec<TurnLifecycleEffect>,
) -> Result<()> {
    if effects.is_empty() {
        return Ok(());
    }
    let lifecycle = RuntimeSessionLifecycle::new(adapter.clone(), org_id, session_id);
    for effect in effects {
        match effect {
            TurnLifecycleEffect::TurnCompleted {
                input_message_id,
                data,
            } => {
                lifecycle
                    .emit_turn_completed(input_message_id, data)
                    .await?;
            }
            TurnLifecycleEffect::SessionIdled {
                turn_id,
                input_message_id,
                iterations,
                usage,
            } => {
                lifecycle
                    .emit_session_idled(turn_id, input_message_id, iterations, usage)
                    .await?;
            }
            TurnLifecycleEffect::TurnFailedWithDisclosure {
                turn_id,
                input_message_id,
                text,
                user_error,
                disclosure,
            } => {
                lifecycle
                    .turn_failed_with_disclosure(
                        turn_id,
                        input_message_id,
                        &text,
                        user_error.as_ref(),
                        disclosure,
                    )
                    .await?;
            }
            TurnLifecycleEffect::FireTurnEndHooks {
                harness_id,
                agent_id,
                turn_id,
                success,
            } => {
                lifecycle
                    .fire_turn_end_hooks(harness_id, agent_id, turn_id, success)
                    .await;
            }
            TurnLifecycleEffect::WaitingForToolResults => {
                lifecycle.waiting_for_tool_results().await?;
            }
        }
    }
    Ok(())
}
