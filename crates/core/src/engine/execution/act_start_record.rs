//! Recording a tool call's start before it runs.
//!
//! Decision: an `AtMostOnce` call does not run unless its `tool.started` was
//! stored. Hosts store that event before `emit` returns (the worker does not
//! queue it), so a log without it proves the call never ran and resume may run
//! it; see `host::turn_resume`. Safe calls run even when the store fails:
//! resume runs them again either way.

use web_time::Instant;

use super::ToolCallResult;
use crate::engine::ExecutionContext;
use crate::engine::events::{EventContext, EventRequest, ToolCompletedData, ToolStartedData};
use crate::engine::tool_fingerprint::tool_error_fingerprint;
use crate::engine::tool_types::{SideEffectClass, ToolResult};
use crate::event_emitter::EventEmitter;

/// Emit `started`. Returns the failed result to report instead of running the
/// call when an `AtMostOnce` call's start could not be stored.
pub(super) async fn record<E: EventEmitter + ?Sized>(
    emitter: &E,
    context: &ExecutionContext,
    event_context: &EventContext,
    started: ToolStartedData,
    tool_start: Instant,
) -> Option<ToolCallResult> {
    let must_record = started.side_effect_class == Some(SideEffectClass::AtMostOnce);
    let tool_call = started.tool_call.clone();
    let fingerprint = started.tool_call_fingerprint.clone();
    let display_name = started.display_name.clone();
    let error = match emitter
        .emit(EventRequest::new(
            context.session_id,
            event_context.clone(),
            started,
        ))
        .await
    {
        Ok(_) => return None,
        Err(error) => error,
    };
    tracing::warn!(
        session_id = %context.session_id,
        tool_call_id = %tool_call.id,
        error = %error,
        "ActAtom: failed to emit tool.started event"
    );
    if !must_record {
        return None;
    }
    let message = format!(
        "tool '{}' was not run because its start could not be recorded: {error}",
        tool_call.name
    );
    let mut completed = ToolCompletedData::failure(
        tool_call.id.clone(),
        tool_call.name.clone(),
        "error".to_string(),
        message.clone(),
        Some(tool_start.elapsed().as_millis() as u64),
    )
    .with_display_name(display_name);
    if let Some(fingerprint) = fingerprint {
        completed = completed.with_fingerprints(
            fingerprint,
            tool_error_fingerprint(&tool_call.name, "error", &message),
        );
    }
    // Best effort: the store that refused the start likely refuses this too.
    let _ = emitter
        .emit(EventRequest::new(
            context.session_id,
            event_context.clone(),
            completed,
        ))
        .await;
    Some(ToolCallResult {
        result: ToolResult {
            details: None,
            tool_call_id: tool_call.id.clone(),
            result: None,
            images: None,
            error: Some(message),
            connection_required: None,
            raw_output: None,
        },
        tool_call,
        success: false,
        status: "error".to_string(),
        connection_required: None,
        determinism_fatal: None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::engine::error::{AgentLoopError, Result};
    use crate::engine::events::Event;
    use crate::engine::tool_types::ToolCall;
    use crate::engine::typed_id::{MessageId, SessionId, TurnId};

    /// Refuses every store and remembers what it was asked to store.
    #[derive(Default)]
    struct Refusing(Mutex<Vec<String>>);

    #[async_trait]
    impl EventEmitter for Refusing {
        async fn emit(&self, request: EventRequest) -> Result<Event> {
            self.0.lock().unwrap().push(request.event_type);
            Err(AgentLoopError::store("pool timed out"))
        }
    }

    async fn record_with(class: Option<SideEffectClass>) -> (Option<ToolCallResult>, Vec<String>) {
        let emitter = Refusing::default();
        let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let started = ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".into(),
                name: "send_email".into(),
                arguments: serde_json::json!({}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: None,
            side_effect_class: class,
        };
        let result = record(
            &emitter,
            &context,
            &EventContext::empty(),
            started,
            Instant::now(),
        )
        .await;
        let stored = emitter.0.lock().unwrap().clone();
        (result, stored)
    }

    #[tokio::test]
    async fn an_at_most_once_call_whose_start_was_not_stored_does_not_run() {
        let (result, stored) = record_with(Some(SideEffectClass::AtMostOnce)).await;
        let result = result.expect("not run");
        assert!(!result.success);
        assert!(
            result
                .result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("start could not be recorded")),
            "{result:?}"
        );
        assert_eq!(stored, vec!["tool.started", "tool.completed"]);
    }

    #[tokio::test]
    async fn a_safe_call_runs_even_when_its_start_was_not_stored() {
        for class in [
            Some(SideEffectClass::Pure),
            Some(SideEffectClass::Idempotent),
            None,
        ] {
            let (result, stored) = record_with(class).await;
            assert!(result.is_none());
            assert_eq!(stored, vec!["tool.started"]);
        }
    }
}
