// The sans-IO turn planner (EVE-840, Sans-IO Turn State epic).
//
// This is the authoritative turn-planning brain, extracted verbatim from
// `everruns-host`'s `plan_next_host_turn`. Every function here is pure and
// deterministic: it reads only its arguments and returns a `TurnPlan` (plus, for
// terminal outcomes, a list of `TurnLifecycleEffect`s the host must perform). It
// never touches a store, socket, process, event bus, or `Utc::now()` — the host
// resolves those facts, passes `now` in, and performs the returned effects.

use chrono::{DateTime, Utc};
use everruns_core::atoms::{ActInput, AtomContext, ReasonResult};
use everruns_core::events::{TokenUsage, TurnCompletedData};
use everruns_core::turn::TurnStopReason;
use everruns_provider::typed_id::{
    AgentId, ExecId, HarnessId, MessageId, SessionId, TurnId, WorkspaceId,
};
use everruns_provider::user_facing_error::codes as user_facing_error_codes;
use everruns_provider::user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, classify_runtime_error_message,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Host-owned state carried across turn phases.
///
/// Durable hosts can persist this between activities; in-memory hosts can hold
/// it directly in memory. The type itself is engine-level and has no host,
/// store, or durable-engine coupling.
///
/// Hosts are expected to serialize this however they want. `everruns-engine`
/// only defines the fields required to resume the next semantic step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnState {
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cumulative_usage: Option<TokenUsage>,
    #[serde(default)]
    pub tool_call_count: u32,
    #[serde(default)]
    pub llm_call_count: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub time_to_first_token_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_message_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_answer_preview: Option<String>,
}

fn default_iteration() -> u32 {
    1
}

/// Engine-owned act scheduling payload.
///
/// Hosts enqueue or execute this immediately using their own worker model.
#[derive(Debug, Clone)]
pub struct ActPlan {
    pub input: ActInput,
    pub previous_response_id: Option<String>,
    pub iteration: u32,
    pub request_id: Option<String>,
    pub resume_state: Box<TurnState>,
}

/// Generic next-step decision for a host turn.
///
/// This intentionally stops at the semantic boundary:
/// - the engine decides what should happen next
/// - the host decides how to persist, enqueue, retry, or resume it
#[derive(Debug, Clone)]
pub enum TurnPlan {
    ScheduleReason(TurnState),
    ScheduleAct(ActPlan),
    Complete {
        stop_reason: TurnStopReason,
        error: Option<String>,
    },
    WaitForToolResults {
        resume: TurnState,
    },
}

/// A lifecycle side effect the engine decided must be recorded, described as
/// data rather than performed.
///
/// The engine never emits events or fires hooks — it *returns* these, and the
/// host applies them (in list order) through its own lifecycle machinery. This
/// keeps planning deterministic while preserving the exact event stream. These
/// are NOT part of the public [`TurnPlan`]; they travel alongside it.
#[derive(Debug, Clone)]
pub enum TurnLifecycleEffect {
    /// Emit `turn.completed` with the summarized turn fields.
    TurnCompleted {
        input_message_id: MessageId,
        data: TurnCompletedData,
    },
    /// Idle the session and emit `session.idled`.
    SessionIdled {
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: Option<u32>,
        usage: Option<TokenUsage>,
    },
    /// Fail the turn with the already-disclosure-filtered error, emitting
    /// `turn.failed` + `session.idled`.
    TurnFailedWithDisclosure {
        turn_id: TurnId,
        input_message_id: MessageId,
        text: String,
        user_error: Option<UserFacingError>,
        disclosure: Option<ErrorDisclosure>,
    },
    /// Fire the advisory `turn_end` lifecycle hooks.
    FireTurnEndHooks {
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        turn_id: TurnId,
        success: bool,
    },
    /// Mark the session `waiting_for_tool_results`.
    WaitingForToolResults,
}

/// Parsed `act` activity output the planner decides over.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActOutcome {
    pub blocked: bool,
    pub waiting_for_tool_results: bool,
}

/// Session facts the host pre-resolves for the reason→act scheduling case.
///
/// The host fetches these (from its session store) only when
/// [`reason_schedules_act`] is true, mirroring the original conditional fetch:
/// `blueprint_id` scopes blueprint tool resolution, and `workspace_id` points
/// tool file I/O at the (possibly shared) workspace rather than the session's
/// own keyspace.
#[derive(Debug, Clone, Default)]
pub struct ActSchedulingFacts {
    pub blueprint_id: Option<String>,
    pub workspace_id: Option<WorkspaceId>,
}

/// Typed, parsed activity output the engine plans the next step from.
///
/// The host parses the raw serialized activity output into this before calling
/// [`plan_next_turn`]; unknown activity kinds are rejected by the host, so the
/// engine stays total.
pub enum ActivityOutcome {
    ProcessInput { turn_id: Option<TurnId> },
    // Boxed: `ReasonResult` dwarfs the other variants (clippy::large_enum_variant).
    Reason(Box<ReasonResult>),
    Act(ActOutcome),
}

/// Host-resolved facts the engine needs but cannot fetch itself.
///
/// The host populates only the field relevant to the completed activity, doing
/// I/O in exactly the same conditions as the original planner:
/// `act_scheduling` only when [`reason_schedules_act`] is true, and
/// `setup_connection_hint_enabled` only when the act paused for tool results.
#[derive(Debug, Clone, Default)]
pub struct HostFacts {
    pub act_scheduling: Option<ActSchedulingFacts>,
    pub setup_connection_hint_enabled: bool,
}

fn preview_final_answer(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    Some(text.chars().take(2000).collect())
}

fn add_usage(current: &mut Option<TokenUsage>, next: &TokenUsage) {
    match current {
        Some(current) => current.add(next),
        None => *current = Some(next.clone()),
    }
}

impl TurnState {
    fn with_reason_summary(&self, reason_result: &ReasonResult) -> Self {
        let mut next = self.clone();
        next.llm_call_count = next.llm_call_count.saturating_add(1);
        next.tool_call_count = next
            .tool_call_count
            .saturating_add(reason_result.tool_calls.len() as u32);
        if let Some(usage) = &reason_result.usage {
            add_usage(&mut next.cumulative_usage, usage);
        }
        if next.time_to_first_token_ms.is_none() {
            next.time_to_first_token_ms = reason_result.time_to_first_token_ms;
        }
        next.final_message_id = reason_result.output_message_id;
        next.final_answer_preview = preview_final_answer(&reason_result.text);
        next
    }

    /// Wall-clock duration since `started_at`, measured against the host-supplied
    /// `now` so the calculation stays deterministic.
    fn duration_ms(&self, now: DateTime<Utc>) -> Option<u64> {
        self.started_at
            .map(|started_at| now.signed_duration_since(started_at))
            .and_then(|duration| u64::try_from(duration.num_milliseconds()).ok())
    }
}

fn classify_reason_failure(reason_result: &ReasonResult) -> UserFacingError {
    // The reason atom already classified and disclosure-filtered the failure.
    // Reuse it so the turn.failed event matches what the session message
    // showed; re-classifying strings here could leak past a generic mode.
    if let Some(user_error) = &reason_result.user_facing_error {
        return user_error.clone();
    }

    let from_text =
        classify_runtime_error_message(&reason_result.text, &UserFacingErrorContext::default());

    let Some(error) = reason_result.error.as_deref() else {
        return from_text;
    };

    let from_error = classify_runtime_error_message(error, &UserFacingErrorContext::default());

    if from_error.code == user_facing_error_codes::PROCESSING_ERROR {
        return from_text;
    }

    if from_error.code == from_text.code
        && from_error.fields.is_empty()
        && !from_text.fields.is_empty()
    {
        return from_text;
    }

    from_error
}

/// Does this reason outcome schedule an act phase?
///
/// The host consults this predicate to decide whether to resolve
/// [`ActSchedulingFacts`] (a session fetch) before calling [`plan_after_reason`]
/// — the same condition under which the original planner fetched the session.
/// The reason planner branches on this same function, so the rule has exactly
/// one definition.
pub fn reason_schedules_act(state: &TurnState, reason_result: &ReasonResult) -> bool {
    let max_turn_requests_reached = state.iteration >= reason_result.max_iterations as u32;
    reason_result.has_tool_calls && reason_result.success && !max_turn_requests_reached
}

/// Plan the next host step after an activity finishes.
///
/// The authoritative, deterministic turn-planning entry point. Given the carried
/// [`TurnState`], the parsed [`ActivityOutcome`], the count of queued steering
/// messages, the host-supplied `now`, and any [`HostFacts`] the host pre-resolved,
/// it returns the [`TurnPlan`] together with the [`TurnLifecycleEffect`]s the
/// host must perform (in order). It performs no I/O of its own.
pub fn plan_next_turn(
    state: &TurnState,
    outcome: ActivityOutcome,
    pending_user_message_count: usize,
    now: DateTime<Utc>,
    facts: HostFacts,
) -> (TurnPlan, Vec<TurnLifecycleEffect>) {
    match outcome {
        ActivityOutcome::ProcessInput { turn_id } => {
            (plan_after_process_input(state, turn_id, now), Vec::new())
        }
        ActivityOutcome::Reason(reason_result) => plan_after_reason(
            state,
            *reason_result,
            pending_user_message_count,
            now,
            facts.act_scheduling,
        ),
        ActivityOutcome::Act(outcome) => {
            plan_after_act(state, outcome, facts.setup_connection_hint_enabled)
        }
    }
}

/// Plan the reason step that follows a completed `process_input` activity.
pub fn plan_after_process_input(
    state: &TurnState,
    turn_id: Option<TurnId>,
    now: DateTime<Utc>,
) -> TurnPlan {
    let next = TurnState {
        turn_id,
        previous_response_id: None,
        iteration: 1,
        started_at: state.started_at.or(Some(now)),
        ..state.clone()
    };
    debug!(session_id = %state.session_id, turn_id = ?turn_id, "planned reason step");
    TurnPlan::ScheduleReason(next)
}

/// Plan the next step after a `reason` activity finishes.
///
/// When [`reason_schedules_act`] holds, `act_scheduling` supplies the session
/// facts the host resolved for the act phase; it is ignored otherwise. A
/// terminal reason outcome returns the lifecycle effects the host must perform;
/// the continuing outcomes return an empty effect list.
pub fn plan_after_reason(
    state: &TurnState,
    reason_result: ReasonResult,
    pending_user_message_count: usize,
    now: DateTime<Utc>,
    act_scheduling: Option<ActSchedulingFacts>,
) -> (TurnPlan, Vec<TurnLifecycleEffect>) {
    let response_id = reason_result.response_id.clone();
    let summarized_state = state.with_reason_summary(&reason_result);
    let max_turn_requests_reached = state.iteration >= reason_result.max_iterations as u32;

    if reason_schedules_act(state, &reason_result) {
        let facts = act_scheduling.unwrap_or_default();
        let plan = ActPlan {
            input: ActInput {
                org_id: Some(state.org_id),
                context: AtomContext {
                    session_id: state.session_id,
                    turn_id: state.turn_id.unwrap_or_default(),
                    input_message_id: state.input_message_id,
                    exec_id: ExecId::new(),
                    workspace_id: facts.workspace_id,
                },
                harness_id: state.harness_id,
                agent_id: state.agent_id,
                tool_calls: reason_result.tool_calls,
                tool_definitions: reason_result.tool_definitions,
                locale: reason_result.locale,
                blueprint_id: facts.blueprint_id,
                network_access: reason_result.network_access,
                // Request-level parallel tool calling preference, carried
                // from agent config through the reason path (EVE-598).
                parallel_tool_calls: reason_result.parallel_tool_calls,
            },
            previous_response_id: response_id,
            iteration: state.iteration,
            request_id: state.request_id.clone(),
            resume_state: Box::new(summarized_state),
        };
        return (TurnPlan::ScheduleAct(plan), Vec::new());
    }

    if reason_result.success && pending_user_message_count > 0 && !max_turn_requests_reached {
        if pending_user_message_count > 1 {
            info!(
                session_id = %state.session_id,
                pending_user_message_count,
                "multiple steering messages arrived during turn"
            );
        }

        let next = TurnState {
            previous_response_id: response_id,
            iteration: state.iteration.saturating_add(1),
            ..summarized_state
        };
        return (TurnPlan::ScheduleReason(next), Vec::new());
    }

    let turn_id = state.turn_id.unwrap_or_default();
    let mut effects = Vec::new();

    if reason_result.success {
        effects.push(TurnLifecycleEffect::TurnCompleted {
            input_message_id: state.input_message_id,
            data: TurnCompletedData {
                turn_id,
                iterations: state.iteration,
                duration_ms: summarized_state.duration_ms(now),
                usage: summarized_state.cumulative_usage.clone(),
                input_content: None,
                final_message_id: summarized_state.final_message_id,
                final_answer_preview: summarized_state.final_answer_preview.clone(),
                time_to_first_token_ms: summarized_state.time_to_first_token_ms,
                tool_call_count: Some(summarized_state.tool_call_count),
                llm_call_count: Some(summarized_state.llm_call_count),
                status: Some("completed".to_string()),
            },
        });
        effects.push(TurnLifecycleEffect::SessionIdled {
            turn_id,
            input_message_id: state.input_message_id,
            iterations: Some(state.iteration),
            usage: summarized_state.cumulative_usage.clone(),
        });
    } else {
        let user_error = classify_reason_failure(&reason_result);
        effects.push(TurnLifecycleEffect::TurnFailedWithDisclosure {
            turn_id,
            input_message_id: state.input_message_id,
            text: reason_result.text.clone(),
            user_error: Some(user_error),
            disclosure: reason_result.error_disclosure,
        });
    }

    // turn_end lifecycle hooks (advisory). Fired once the turn reaches a
    // terminal reason outcome on the durable/strategy path.
    effects.push(TurnLifecycleEffect::FireTurnEndHooks {
        harness_id: state.harness_id,
        agent_id: state.agent_id,
        turn_id,
        success: reason_result.success,
    });

    let stop_reason = if !reason_result.success {
        match TurnStopReason::from_provider_finish_reason(reason_result.finish_reason.as_deref()) {
            TurnStopReason::Refusal => TurnStopReason::Refusal,
            _ => TurnStopReason::Error,
        }
    } else if max_turn_requests_reached
        && (reason_result.has_tool_calls || pending_user_message_count > 0)
    {
        TurnStopReason::MaxTurnRequests
    } else {
        TurnStopReason::from_provider_finish_reason(reason_result.finish_reason.as_deref())
    };

    (
        TurnPlan::Complete {
            stop_reason,
            error: reason_result.error,
        },
        effects,
    )
}

/// Plan the next step after an `act` activity finishes.
///
/// `setup_connection_hint_enabled` is the resolved session hint; the host reads
/// it only when the act reported `waiting_for_tool_results`, so passing `false`
/// otherwise matches the original short-circuit exactly.
pub fn plan_after_act(
    state: &TurnState,
    outcome: ActOutcome,
    setup_connection_hint_enabled: bool,
) -> (TurnPlan, Vec<TurnLifecycleEffect>) {
    if outcome.blocked {
        return (
            TurnPlan::Complete {
                stop_reason: TurnStopReason::EndTurn,
                error: None,
            },
            Vec::new(),
        );
    }

    let should_pause_for_tool_results =
        outcome.waiting_for_tool_results && setup_connection_hint_enabled;

    let next = TurnState {
        iteration: state.iteration.saturating_add(1),
        ..state.clone()
    };

    if should_pause_for_tool_results {
        return (
            TurnPlan::WaitForToolResults { resume: next },
            vec![TurnLifecycleEffect::WaitingForToolResults],
        );
    }

    if outcome.waiting_for_tool_results {
        info!(
            session_id = %state.session_id,
            "setup_connection hint absent, continuing turn instead of pausing"
        );
    }

    (TurnPlan::ScheduleReason(next), Vec::new())
}
