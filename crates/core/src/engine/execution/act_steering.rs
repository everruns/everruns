//! Record skipped calls without creating a tool-start event.

use std::sync::Arc;

use super::{
    ActAtom, EventContext, EventEmitter, EventRequest, ExecutionContext, Result, ToolCall,
    ToolCallResult, ToolCompletedData, ToolExecutor, ToolResult, Uuid, tool_result_fingerprint,
};

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    pub(super) async fn record_redirect_skip(
        &self,
        context: &ExecutionContext,
        event_context: EventContext,
        tool_call: ToolCall,
        tool_call_fingerprint: String,
        display_name: Option<String>,
        claim_token: Option<Uuid>,
    ) -> Result<ToolCallResult> {
        let result = ToolResult {
            tool_call_id: tool_call.id.clone(),
            details: Some(
                serde_json::json!({"execution_status":"skipped", "reason":"explicit_redirect"}),
            ),
            ..ToolResult::error(
                "Not executed: the user explicitly redirected this turn before this call was admitted. Follow the new instruction at the next reason boundary.",
            )
        };
        if let Some((store, token)) = self.durable_tool_result_store.as_ref().zip(claim_token) {
            let settled = store
                .settle_tool_call(
                    &context.turn_id.to_string(),
                    &tool_call.id,
                    serde_json::to_value(&result)
                        .map_err(|e| crate::engine::error::AgentLoopError::store(e.to_string()))?,
                    "settled",
                    token,
                )
                .await?;
            if !settled {
                return Err(crate::engine::error::AgentLoopError::store(
                    "redirect skip lost its durable claim",
                ));
            }
        }
        // No tool.started: no invocation took place. The completed error is
        // paired to the model-authored call by id, never a fabricated success.
        self.event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context,
                ToolCompletedData::from_tool_result(&tool_call, &result, "skipped", None)
                    .with_fingerprints(
                        tool_call_fingerprint,
                        tool_result_fingerprint(&tool_call.name, &result),
                    )
                    .with_display_name(display_name),
            ))
            .await?;
        Ok(ToolCallResult {
            tool_call,
            result,
            success: false,
            status: "skipped".into(),
            connection_required: None,
            determinism_fatal: None,
        })
    }
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: crate::engine::phase_effects::PhaseEffectSink,
{
    /// Check explicit redirection only as a queued server call is admitted.
    pub fn with_tool_batch_steering(
        mut self,
        steering: Option<Arc<dyn crate::engine::tool_steering::ToolBatchSteering>>,
    ) -> Self {
        self.tool_batch_steering = steering;
        self
    }
}
