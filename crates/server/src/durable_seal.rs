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

use crate::kernel_imports::{
    Caller, Message, TURN_STARTED, everruns_provider::user_facing_error::UserFacingError,
};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    EventContext, EventData, EventRequest, OutputMessageCompletedData, SessionIdledData,
    TurnSealedData,
};
use everruns_durable::SealedTaskInfo;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use everruns_provider::user_facing_error::codes as user_facing_error_codes;

use crate::domains::sessions::SessionService;
use crate::services::EventService;

/// Session context needed to emit sealed-turn events.
struct SealedSessionContext {
    org_id: i64,
    session_id: SessionId,
    turn_id: Option<TurnId>,
    input_message_id: MessageId,
}

/// Best-effort extraction of session context from a task input.
///
/// `process_input`/`reason` tasks serialize engine `TurnState` with the IDs at
/// the top level; `act` tasks nest them under `context`. Try both shapes.
fn extract_context(input: &serde_json::Value) -> Option<SealedSessionContext> {
    let org_id = input.get("org_id").and_then(|v| v.as_i64())?;

    // Top-level shape (TurnState / DurableTurnInput).
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
    // Initial process_input tasks can be sealed before their worker persists a
    // generated turn_id. Keep session-level context so the seal can still
    // release the active session; only turn-scoped events require a real turn.
    let turn_id = turn_id_v.and_then(parse_id::<TurnId>);

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

async fn recover_turn_id_from_events(
    event_service: &EventService,
    ctx: &SealedSessionContext,
    sealed: &SealedTaskInfo,
) -> Option<TurnId> {
    let filter_types = [TURN_STARTED.to_string()];
    let events = match event_service
        .list(
            ctx.session_id.uuid(),
            None,
            None,
            &filter_types,
            &[],
            None,
            Some(100),
        )
        .await
    {
        Ok(events) => events,
        Err(e) => {
            tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to recover turn_id from turn.started events");
            return None;
        }
    };

    events.into_iter().rev().find_map(|event| match event.data {
        EventData::TurnStarted(data) if data.input_message_id == ctx.input_message_id => {
            Some(data.turn_id)
        }
        _ => None,
    })
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
            workflow_id = ?sealed.workflow_id,
            activity_type = %sealed.activity_type,
            "Cannot surface sealed turn: missing/unparseable session context in task input"
        );
        return;
    };

    let turn_id = match ctx.turn_id {
        Some(turn_id) => Some(turn_id),
        None => recover_turn_id_from_events(event_service, &ctx, sealed).await,
    };

    if let Some(turn_id) = turn_id {
        let context = EventContext::turn(turn_id, ctx.input_message_id);

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
                    turn_id,
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
                    turn_id,
                    iterations: None,
                    usage: None,
                },
            ))
            .await
        {
            tracing::warn!(task_id = %sealed.task_id, error = %e, "Failed to emit session.idled after seal");
        }
    } else {
        tracing::warn!(
            task_id = %sealed.task_id,
            workflow_id = ?sealed.workflow_id,
            activity_type = %sealed.activity_type,
            "Sealed task has no recoverable turn_id; skipping turn-scoped events but idling session"
        );
    }

    // 4) Set session status to idle even when an initial process_input task was
    // sealed before it persisted a turn_id. This is the availability-critical
    // side effect that prevents the session from staying wedged active.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_context_parses_top_level_shape() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        let input = serde_json::json!({
            "org_id": 7,
            "session_id": session_id.to_string(),
            "turn_id": turn_id.to_string(),
            "input_message_id": message_id.to_string(),
        });

        let ctx = extract_context(&input).expect("should parse");
        assert_eq!(ctx.org_id, 7);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, Some(turn_id));
        assert_eq!(ctx.input_message_id, message_id);
    }

    #[test]
    fn extract_context_preserves_session_when_turn_id_missing() {
        let session_id = SessionId::new();
        let message_id = MessageId::new();
        let input = serde_json::json!({
            "org_id": 7,
            "session_id": session_id.to_string(),
            "input_message_id": message_id.to_string(),
        });

        let ctx = extract_context(&input).expect("session context should parse");
        assert_eq!(ctx.org_id, 7);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, None);
        assert_eq!(ctx.input_message_id, message_id);
    }

    #[test]
    fn extract_context_preserves_session_when_turn_id_unparseable() {
        let session_id = SessionId::new();
        let message_id = MessageId::new();
        let input = serde_json::json!({
            "org_id": 7,
            "session_id": session_id.to_string(),
            "turn_id": "not-a-valid-id",
            "input_message_id": message_id.to_string(),
        });

        let ctx = extract_context(&input).expect("session context should parse");
        assert_eq!(ctx.org_id, 7);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, None);
        assert_eq!(ctx.input_message_id, message_id);
    }
}
