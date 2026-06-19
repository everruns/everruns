//! Surface sealed durable turns to the session (EVE-534).
//!
//! When the stale-task reclaim path seals a non-progressing turn (see
//! `everruns_durable::SealedTaskInfo` and the forward-progress guard in the
//! durable store), the control plane must make that seal observable:
//!
//! - emit a `turn.sealed` session event (distinct from `turn.completed` and
//!   `turn.failed`),
//! - emit a user-facing assistant message explaining the stop,
//! - emit `session.idled` and set the session status to `idle` so the UI
//!   unblocks and the user can start a new turn.
//!
//! Session status: a sealed turn ends the turn, so the session returns to
//! `idle` (the same terminal status used for `turn.failed`). The Sealed state is
//! observably distinct via the `turn.sealed` event and its `reason`, not via a
//! dedicated session status — there is no `sealed` session status today and the
//! worker gRPC contract only accepts the existing four statuses.

use std::sync::Arc;

use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionIdledData, TurnSealedData,
};
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::{Caller, Message, UserFacingError, user_facing_error_codes};
use everruns_durable::SealedTaskInfo;

use crate::domains::sessions::SessionService;
use crate::services::EventService;

/// Session context needed to emit sealed-turn events.
struct SealedSessionContext {
    org_id: i64,
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
}

/// Best-effort extraction of session context from a task input.
///
/// `process_input`/`reason` tasks serialize `RuntimeTurnState` with the IDs at
/// the top level; `act` tasks nest them under `context`. Try both shapes.
fn extract_context(input: &serde_json::Value) -> Option<SealedSessionContext> {
    let org_id = input.get("org_id").and_then(|v| v.as_i64())?;

    // Top-level shape (RuntimeTurnState / DurableTurnInput).
    let top = (
        input.get("session_id"),
        input.get("turn_id"),
        input.get("input_message_id"),
    );
    // Nested shape (ActInput.context).
    let nested = input.get("context").map(|c| {
        (
            c.get("session_id"),
            c.get("turn_id"),
            c.get("input_message_id"),
        )
    });

    let (session_id_v, turn_id_v, input_message_id_v) = match top {
        (Some(s), _, Some(m)) => (Some(s), top.1, Some(m)),
        _ => nested?,
    };

    let session_id = session_id_v.and_then(parse_id::<SessionId>)?;
    let input_message_id = input_message_id_v.and_then(parse_id::<MessageId>)?;
    // turn_id may be absent on a brand-new turn; fall back to a fresh id so the
    // events still correlate to *something* rather than being dropped.
    let turn_id = turn_id_v
        .and_then(parse_id::<TurnId>)
        .unwrap_or_else(TurnId::new);

    Some(SealedSessionContext {
        org_id,
        session_id,
        turn_id,
        input_message_id,
    })
}

fn parse_id<T: std::str::FromStr>(v: &serde_json::Value) -> Option<T> {
    v.as_str().and_then(|s| s.parse::<T>().ok())
}

/// Emit the user-facing seal events for one sealed task and idle the session.
///
/// Best-effort: each step logs on failure but never panics, mirroring the
/// reclaim loop's tolerance for partial failures.
pub async fn handle_sealed_task(
    event_service: &Arc<EventService>,
    session_service: &Arc<SessionService>,
    sealed: &SealedTaskInfo,
) {
    let Some(ctx) = extract_context(&sealed.input) else {
        tracing::warn!(
            task_id = %sealed.task_id,
            activity_type = %sealed.activity_type,
            "Cannot surface sealed turn: failed to parse session context from task input"
        );
        return;
    };

    let context = EventContext::turn(ctx.turn_id, ctx.input_message_id);

    // 1) turn.sealed — the canonical, distinct terminal event.
    let detail = format!(
        "No forward progress across {} consecutive recoveries; sealed to prevent a crash-loop.",
        sealed.no_progress_count
    );
    if let Err(e) = event_service
        .emit(EventRequest::new(
            ctx.session_id,
            context.clone(),
            TurnSealedData {
                turn_id: ctx.turn_id,
                reason: sealed.reason.clone(),
                detail: Some(detail.clone()),
                iterations: None,
                usage: None,
            },
        ))
        .await
    {
        tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to emit turn.sealed event");
    }

    // 2) User-facing assistant message so the conversation shows the stop.
    let user_error = UserFacingError::new(user_facing_error_codes::PROCESSING_ERROR);
    let mut message = Message::assistant(
        "This turn was stopped because it repeatedly failed without making progress.",
    );
    let mut metadata = std::collections::HashMap::new();
    user_error.apply_to_message_metadata(&mut metadata);
    message.metadata = Some(metadata);
    if let Err(e) = event_service
        .emit(EventRequest::new(
            ctx.session_id,
            context.clone(),
            OutputMessageCompletedData::new(message).with_user_facing_error(&user_error),
        ))
        .await
    {
        tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to emit sealed message");
    }

    // 3) session.idled so the UI unblocks.
    if let Err(e) = event_service
        .emit(EventRequest::new(
            ctx.session_id,
            context,
            SessionIdledData {
                turn_id: ctx.turn_id,
                iterations: None,
                usage: None,
            },
        ))
        .await
    {
        tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to emit session.idled after seal");
    }

    // 4) Set session status to idle.
    let caller = Caller::internal(ctx.org_id);
    if let Err(e) = session_service
        .update_status(&caller, ctx.session_id.uuid(), "idle".to_string())
        .await
    {
        tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to idle session after seal");
    }

    tracing::info!(
        task_id = %sealed.task_id,
        workflow_id = ?sealed.workflow_id,
        reason = %sealed.reason,
        no_progress_count = sealed.no_progress_count,
        "Surfaced sealed turn to session (EVE-534)"
    );
}
