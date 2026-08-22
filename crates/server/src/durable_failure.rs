//! Surface exhausted durable task failures to their session.

use std::sync::Arc;

use crate::domains::sessions::SessionService;
use crate::durable_seal::{DurableTaskSessionContext, extract_context};
use crate::kernel_imports::{Caller, Message, TURN_STARTED};
use crate::services::EventService;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    EventContext, EventData, EventRequest, OutputMessageCompletedData, SessionIdledData,
    TurnFailedData,
};
use everruns_durable::DeadTaskInfo;
use everruns_provider::typed_id::TurnId;
use everruns_provider::user_facing_error::{
    UserFacingErrorContext, classify_runtime_error_message,
};

async fn recover_turn_id_from_events(
    event_service: &EventService,
    ctx: &DurableTaskSessionContext,
    task_id: uuid::Uuid,
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
        Err(error) => {
            tracing::warn!(%task_id, %error, "Failed to recover turn_id for exhausted task");
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

/// Emit the canonical failed-turn lifecycle for the CAS winner.
pub async fn handle_failed_task(
    event_service: &Arc<EventService>,
    session_service: &Arc<SessionService>,
    task: &DeadTaskInfo,
    error: &str,
) {
    let Some(ctx) = extract_context(&task.input) else {
        tracing::warn!(
            task_id = %task.task_id,
            workflow_id = ?task.workflow_id,
            activity_type = %task.activity_type,
            "Cannot surface exhausted task: missing session context"
        );
        return;
    };
    let turn_id = match ctx.turn_id {
        Some(turn_id) => Some(turn_id),
        None => recover_turn_id_from_events(event_service, &ctx, task.task_id).await,
    };

    if let Some(turn_id) = turn_id {
        let error_chain = error.split("error_chain=").nth(1).unwrap_or(error).trim();
        let user_error =
            classify_runtime_error_message(error_chain, &UserFacingErrorContext::default());
        let shown = user_error.fallback_message();
        let context = EventContext::turn(turn_id, ctx.input_message_id);

        let mut message = Message::assistant(shown.clone());
        let mut metadata = std::collections::HashMap::new();
        user_error.apply_to_message_metadata(&mut metadata);
        message.metadata = Some(metadata);
        if let Err(error) = event_service
            .emit(EventRequest::new(
                ctx.session_id,
                context.clone(),
                OutputMessageCompletedData::new(message).with_user_facing_error(&user_error),
            ))
            .await
        {
            tracing::warn!(task_id = %task.task_id, %error, "Failed to emit exhausted-task message");
        }

        let mut failure = TurnFailedData {
            turn_id,
            error: shown,
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        };
        user_error.apply_to_event_fields(&mut failure.error_code, &mut failure.error_fields);
        if let Err(error) = event_service
            .emit(EventRequest::new(ctx.session_id, context.clone(), failure))
            .await
        {
            tracing::warn!(task_id = %task.task_id, %error, "Failed to emit turn.failed for exhausted task");
        }

        if let Err(error) = event_service
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
            tracing::warn!(task_id = %task.task_id, %error, "Failed to emit session.idled for exhausted task");
        }
    }

    let caller = Caller::internal(ctx.org_id);
    if let Err(error) = session_service
        .update_status(&caller, ctx.session_id.uuid(), "idle".to_string())
        .await
    {
        tracing::warn!(task_id = %task.task_id, %error, "Failed to idle session after exhausted task");
    }
}
