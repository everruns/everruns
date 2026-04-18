// Shared turn-strategy planning for embedded, durable, and custom hosts.
// Decision: everruns-runtime stays durable-agnostic and returns generic next-step plans.

use crate::{RuntimeHostAdapter, RuntimeSessionLifecycle};
use everruns_core::atoms::{ActInput, AtomContext};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::typed_id::{AgentId, ExecId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{Controls, ReasonResult};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Host-owned state carried across turn phases.
///
/// Durable hosts can persist this between activities; in-memory hosts can hold
/// it directly in memory. The type itself is runtime-level and has no durable
/// engine coupling.
///
/// Hosts are expected to serialize this however they want. `everruns-runtime`
/// only defines the fields required to resume the next semantic step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTurnState {
    pub org_id: i64,
    pub session_id: SessionId,
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub input_message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub previous_response_id: Option<String>,
    #[serde(default = "default_iteration")]
    pub iteration: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

fn default_iteration() -> u32 {
    1
}

/// Runtime-owned act scheduling payload.
///
/// Hosts enqueue or execute this immediately using their own worker model.
#[derive(Debug, Clone)]
pub struct RuntimeActPlan {
    pub input: ActInput,
    pub previous_response_id: Option<String>,
    pub iteration: u32,
    pub request_id: Option<String>,
}

/// Generic next-step decision for a host turn.
///
/// This intentionally stops at the semantic boundary:
/// - runtime decides what should happen next
/// - the host decides how to persist, enqueue, retry, or resume it
#[derive(Debug, Clone)]
pub enum RuntimeTurnPlan {
    ScheduleReason(RuntimeTurnState),
    ScheduleAct(RuntimeActPlan),
    Complete { error: Option<String> },
    WaitForToolResults { resume: RuntimeTurnState },
}

/// Determine the next host step after an activity finishes.
///
/// The host provides:
/// - `completed_activity`: which phase just finished
/// - `state`: host-carried turn state
/// - `output`: serialized activity output
/// - `pending_user_message_count`: number of queued steering messages already consumed by the host
///
/// Runtime owns the semantic decision. Hosts translate the returned plan into
/// their own queueing / persistence model.
///
/// Typical host mapping:
/// - `ScheduleReason` => enqueue or invoke a reason phase with the returned state
/// - `ScheduleAct` => enqueue or invoke an act phase with the returned payload
/// - `WaitForToolResults` => persist the resume state until external tool input arrives
/// - `Complete` => mark the host-owned workflow/session turn complete
pub async fn plan_next_host_turn<A: RuntimeHostAdapter>(
    adapter: &A,
    completed_activity: &str,
    state: &RuntimeTurnState,
    output: &serde_json::Value,
    pending_user_message_count: usize,
) -> Result<RuntimeTurnPlan> {
    match completed_activity {
        "process_input" => {
            let turn_id: Option<TurnId> = output
                .get("turn_id")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse().ok());
            let next = RuntimeTurnState {
                turn_id,
                previous_response_id: None,
                iteration: 1,
                ..state.clone()
            };
            debug!(session_id = %state.session_id, turn_id = ?turn_id, "planned reason step");
            Ok(RuntimeTurnPlan::ScheduleReason(next))
        }
        "reason" => {
            let reason_result: ReasonResult = serde_json::from_value(output.clone())
                .map_err(|error| AgentLoopError::Internal(error.into()))?;
            let response_id = reason_result.response_id.clone();

            if reason_result.has_tool_calls && reason_result.success {
                let plan = RuntimeActPlan {
                    input: ActInput {
                        org_id: Some(state.org_id),
                        context: AtomContext {
                            session_id: state.session_id,
                            turn_id: state.turn_id.unwrap_or_default(),
                            input_message_id: state.input_message_id,
                            exec_id: ExecId::new(),
                        },
                        harness_id: state.harness_id,
                        agent_id: state.agent_id,
                        tool_calls: reason_result.tool_calls,
                        tool_definitions: reason_result.tool_definitions,
                        locale: reason_result.locale,
                        blueprint_id: None,
                        network_access: reason_result.network_access,
                    },
                    previous_response_id: response_id,
                    iteration: state.iteration,
                    request_id: state.request_id.clone(),
                };
                return Ok(RuntimeTurnPlan::ScheduleAct(plan));
            }

            if reason_result.success && pending_user_message_count > 0 {
                if pending_user_message_count > 1 {
                    info!(
                        session_id = %state.session_id,
                        pending_user_message_count,
                        "multiple steering messages arrived during turn"
                    );
                }

                let next = RuntimeTurnState {
                    previous_response_id: response_id,
                    iteration: state.iteration.saturating_add(1),
                    ..state.clone()
                };
                return Ok(RuntimeTurnPlan::ScheduleReason(next));
            }

            let lifecycle =
                RuntimeSessionLifecycle::new(adapter.clone(), state.org_id, state.session_id);
            let turn_id = state.turn_id.unwrap_or_default();

            if reason_result.success {
                lifecycle
                    .emit_turn_completed(
                        turn_id,
                        state.input_message_id,
                        state.iteration,
                        reason_result.usage.clone(),
                        None,
                    )
                    .await;
                lifecycle
                    .emit_session_idled(
                        turn_id,
                        state.input_message_id,
                        Some(state.iteration),
                        reason_result.usage.clone(),
                    )
                    .await;
            } else {
                lifecycle
                    .turn_failed(
                        turn_id,
                        state.input_message_id,
                        &reason_result.text,
                        Some("llm_error"),
                    )
                    .await;
            }

            Ok(RuntimeTurnPlan::Complete {
                error: reason_result.error,
            })
        }
        "act" => {
            if output
                .get("blocked")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return Ok(RuntimeTurnPlan::Complete { error: None });
            }

            let waiting_for_tool_results = output
                .get("waiting_for_tool_results")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let should_pause_for_tool_results = waiting_for_tool_results
                && setup_connection_hint_enabled(adapter, state.org_id, state.session_id).await;

            let next = RuntimeTurnState {
                iteration: state.iteration.saturating_add(1),
                ..state.clone()
            };

            if should_pause_for_tool_results {
                let lifecycle =
                    RuntimeSessionLifecycle::new(adapter.clone(), state.org_id, state.session_id);
                lifecycle.waiting_for_tool_results().await;
                return Ok(RuntimeTurnPlan::WaitForToolResults { resume: next });
            }

            if waiting_for_tool_results {
                info!(
                    session_id = %state.session_id,
                    "setup_connection hint absent, continuing turn instead of pausing"
                );
            }

            Ok(RuntimeTurnPlan::ScheduleReason(next))
        }
        other => Err(AgentLoopError::config(format!(
            "Unknown activity type completed: {other}"
        ))),
    }
}

async fn setup_connection_hint_enabled<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
) -> bool {
    match adapter.session_store(org_id).get_session(session_id).await {
        Ok(Some(session)) => {
            let hints = Controls::resolve_hints(session.hints.as_ref(), None);
            hints
                .get("setup_connection")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        }
        _ => false,
    }
}
