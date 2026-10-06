//! Recovery of a reason step that a lost attempt left open (EVE-532).
//!
//! A worker that dies mid-step has already stored what the step announces
//! before its model call: `reason.started`, then `output.message.started`
//! with the message's id. The retry runs the same step, so announcing it
//! again would leave a started message (or step) that never completes.
//!
//! Decisions:
//! - The lookup runs once per step, before `reason.started`, so its answer
//!   decides both whether the step is announced and how the stream recovers.
//! - A dead attempt's step stays open: the retry emits no `reason.started`,
//!   and its `reason.completed` closes the dead attempt's. An attempt that
//!   recorded `reason.completed` itself (it failed, and the engine retries)
//!   closed its step, so the retry announces a fresh one.
//! - Finalize and restart both reuse the partial's message id and emit no
//!   second `output.message.started`: the one already stored is the start of
//!   the message the retry completes. `reason.recovered` is the retry's
//!   observable signal.
//! - Only a dead attempt's partial is finalized. A settled attempt's text
//!   came from a stream that failed, so the retry restarts the call.
//! - The capability-rejection fallback's second provider call starts without
//!   a prior stream: the recovery belongs to the step's first call.

use std::collections::HashMap;

use crate::engine::durability::PartialStreamState;
use crate::engine::error::{AgentLoopError, Result};
use crate::engine::events::{
    EventContext, EventRequest, OutputMessageCompletedData, ReasonRecoveredData, RecoveryMode,
};
use crate::engine::message::RuntimeMessage;
use crate::engine::typed_id::{MessageId, SessionId};

use super::super::ExecutionContext;
use super::error_policy::filter_response_text;
use super::reasoning_updates::{self, ReasoningReplay};
use super::{ReasonAtom, ReasonResult};

/// What the event log holds for this step's turn before the step runs.
pub(super) enum PriorStream {
    /// No stream left open: the first run of this step.
    None,
    /// The partial-stream store could not answer.
    Unknown(AgentLoopError),
    /// A stream an earlier attempt opened and never completed.
    Open(PartialStreamState),
}

/// How the step's first provider call proceeds after consulting the prior.
pub(super) enum Recovery {
    /// Run the call with a newly allocated message id.
    Fresh,
    /// Run the call and complete the open message under its existing id.
    Restart(MessageId),
    /// The open message was completed from its persisted text; no call runs.
    Finalized(Box<ReasonResult>),
}

impl PriorStream {
    /// Whether this attempt announces its step with `reason.started`.
    pub(super) fn announces_step(&self) -> bool {
        !matches!(self, Self::Open(partial) if !partial.attempt_settled)
    }
}

impl ReasonAtom {
    /// Look up the stream an earlier attempt of this step left open, if any.
    pub(super) async fn prior_stream(&self, context: &ExecutionContext) -> PriorStream {
        let Some(store) = self.partial_stream_store.as_ref() else {
            return PriorStream::None;
        };
        let turn_id = context.turn_id.to_string();
        match store.get_partial_stream(context.session_id, &turn_id).await {
            Ok(Some(partial)) => PriorStream::Open(partial),
            Ok(None) => PriorStream::None,
            Err(error) => PriorStream::Unknown(error),
        }
    }

    /// Apply the ContinuePartial policy to `prior` for the step's call.
    pub(super) async fn recover_partial_stream(
        &self,
        prior: PriorStream,
        context: &ExecutionContext,
        iteration: u32,
        runtime_agent: &crate::engine::RuntimeAgent,
        resolved_capability_configs: &[crate::engine::CapabilityRef],
        reasoning_replay: Option<&mut ReasoningReplay>,
    ) -> Result<Recovery> {
        let session_id = context.session_id;
        let partial = match prior {
            PriorStream::None => return Ok(Recovery::Fresh),
            PriorStream::Unknown(error) => {
                if reasoning_replay.is_some() {
                    return Err(error);
                }
                // Best-effort: log and continue with normal execution.
                tracing::warn!(
                    session_id = %session_id,
                    turn_id = %context.turn_id,
                    error = %error,
                    "ReasonAtom: partial-stream store error; proceeding with normal execution"
                );
                return Ok(Recovery::Fresh);
            }
            PriorStream::Open(partial) => partial,
        };
        if !partial.accumulated.is_empty() && !partial.attempt_settled {
            return self
                .finalize_partial_stream(
                    session_id,
                    context,
                    partial,
                    iteration,
                    runtime_agent,
                    resolved_capability_configs,
                )
                .await
                .map(|result| Recovery::Finalized(Box::new(result)));
        }
        if let (Some(replay), Some(mut saved)) = (reasoning_replay, partial.reasoning_state) {
            // The old worker persisted the effective live override before
            // sending. Its process-local handle is gone.
            saved.pending = saved.effective;
            replay.state = saved;
        }
        let _ = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::from_execution_context(context),
                ReasonRecoveredData {
                    turn_id: context.turn_id,
                    mode: RecoveryMode::Restart,
                    accumulated_len: 0,
                },
            ))
            .await;
        tracing::info!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            message_id = %partial.message_id,
            "ReasonAtom: restarting an interrupted stream under its message id"
        );
        Ok(Recovery::Restart(partial.message_id))
    }

    /// Finalize a partial assistant stream without making a new provider call.
    ///
    /// Emits `output.message.completed` from the persisted `accumulated` text
    /// under the partial's message id, and `reason.recovered { mode: Finalize }`.
    async fn finalize_partial_stream(
        &self,
        session_id: SessionId,
        context: &ExecutionContext,
        partial: PartialStreamState,
        iteration: u32,
        runtime_agent: &crate::engine::RuntimeAgent,
        resolved_capability_configs: &[crate::engine::CapabilityRef],
    ) -> Result<ReasonResult> {
        let event_context = EventContext::from_execution_context(context);
        let turn_id = context.turn_id;
        let message_id = partial.message_id;

        // Build the assistant message from capability-filtered accumulated text
        // and persist it via the canonical event path.
        let accumulated = filter_response_text(
            &self.capability_registry,
            resolved_capability_configs,
            partial.accumulated,
        );
        let mut assistant_message = RuntimeMessage::assistant(&accumulated).with_id(message_id);
        if let Some(state) = partial.reasoning_state {
            assistant_message.metadata = Some(HashMap::from([
                ("model".into(), serde_json::json!("gpt-6-astra")),
                ("provider".into(), serde_json::json!("openai")),
                (
                    reasoning_updates::STATE_KEY.into(),
                    serde_json::json!(state),
                ),
                (
                    "reasoning_effort".into(),
                    serde_json::json!(state.effective),
                ),
            ]));
        }
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                OutputMessageCompletedData::new(assistant_message),
            ))
            .await?;

        // Emit observability event.
        let accumulated_len = accumulated.len();
        let _ = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                ReasonRecoveredData {
                    turn_id,
                    mode: RecoveryMode::Finalize,
                    accumulated_len,
                },
            ))
            .await;

        tracing::info!(
            session_id = %session_id,
            turn_id = %turn_id,
            iteration,
            accumulated_len,
            "ReasonAtom: finalized partial stream from persisted accumulated text"
        );

        Ok(ReasonResult {
            native_counts: None,
            success: true,
            text: accumulated,
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: runtime_agent.tools.clone(),
            max_iterations: runtime_agent.max_iterations,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: Some(message_id),
            time_to_first_token_ms: None,
            response_id: None,
            finish_reason: Some("stop".to_string()),
            // No locale, network list, or parallel-call preference on finalize.
            ..ReasonResult::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(accumulated: &str, attempt_settled: bool) -> PriorStream {
        PriorStream::Open(PartialStreamState {
            reasoning_state: None,
            message_id: MessageId::new(),
            accumulated: accumulated.to_string(),
            attempt_settled,
        })
    }

    #[test]
    fn a_dead_attempt_keeps_its_step_announced_once() {
        assert!(!open("", false).announces_step());
        assert!(!open("partial", false).announces_step());
    }

    #[test]
    fn a_first_run_a_settled_attempt_or_an_unknown_prior_announces_the_step() {
        assert!(PriorStream::None.announces_step());
        assert!(open("", true).announces_step());
        assert!(open("partial", true).announces_step());
        assert!(PriorStream::Unknown(AgentLoopError::store("down")).announces_step());
    }
}
