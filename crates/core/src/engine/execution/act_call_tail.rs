//! The rest of one tool call once its pre-tool hooks ran: invoke the tool,
//! run the post-tool hooks, record the call's `tool.completed` and settle its
//! durable claim.
//!
//! Decision: the tail owns everything it reads (hooks, narration, emitter,
//! the call's context), so it can run as its own task. A call that runs past
//! its host's detach threshold leaves its act this way and finishes after the
//! turn parked (see `tool_detach`); every other call awaits the same tail in
//! place.

use std::sync::Arc;

use uuid::Uuid;
use web_time::Instant;

use super::narration::Narrator;
use super::{ToolCallResult, act_hooks, client_policy};
use crate::engine::ExecutionContext;
use crate::engine::durability::DurableToolResultStore;
use crate::engine::error::Result;
use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{EventContext, EventRequest, ToolCompletedData};
use crate::engine::tool_context::ToolContext;
use crate::engine::tool_execution::ToolExecutor;
use crate::engine::tool_fingerprint::{tool_error_fingerprint, tool_result_fingerprint};
use crate::engine::tool_narration::ToolNarrationPhase;
use crate::engine::tool_types::{ToolCall, ToolDefinition, ToolResult};

pub(super) struct CallTail {
    pub(super) emitter: Arc<dyn EventEmitter>,
    pub(super) post_tool_hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
    pub(super) final_post_tool_hooks: Vec<Arc<dyn act_hooks::PostToolExecHook>>,
    pub(super) narrator: Narrator,
    /// The durable claim (EVE-530) to settle, with its token.
    pub(super) claim: Option<(Arc<dyn DurableToolResultStore>, Uuid)>,
    pub(super) context: ExecutionContext,
    pub(super) event_context: EventContext,
    /// The call as the model authored it.
    pub(super) tool_call: ToolCall,
    /// The call as it runs, after the pre-tool hooks rewrote it.
    pub(super) execution_tool_call: ToolCall,
    /// What `tool.completed` records as the authored arguments (EVE-1209).
    pub(super) authored_arguments: serde_json::Value,
    pub(super) tool_def: ToolDefinition,
    pub(super) tool_context: ToolContext,
    pub(super) locale: Option<String>,
    pub(super) tool_start: Instant,
    pub(super) tool_call_fingerprint: String,
    pub(super) display_name: Option<String>,
    pub(super) capability_id: Option<String>,
    pub(super) capability_name: Option<String>,
    /// Cancels the call's tool work when the call ends, however it ends.
    pub(super) _cancel_on_call_end: tokio_util::sync::DropGuard,
}

impl CallTail {
    /// Invoke the tool on this task, then finish the call. Not for
    /// `cpu_bound` tools, which the act runs on a task of their own.
    pub(super) async fn run<T: ToolExecutor + ?Sized>(
        self,
        executor: Arc<T>,
        clock: Option<Arc<dyn super::super::cpu_time::ThreadCpuClock>>,
    ) -> ToolCallResult {
        let (result, cpu_ns) = super::super::cpu_time::observe_cpu(
            executor.execute_with_context(
                &self.execution_tool_call,
                &self.tool_def,
                &self.tool_context,
            ),
            clock.as_deref(),
        )
        .await;
        self.finish(result, cpu_ns).await
    }

    fn narration(&self, phase: ToolNarrationPhase, outcome: Option<&ToolResult>) -> Option<String> {
        Some(self.narrator.render(
            Some(&self.tool_def),
            &self.tool_call,
            phase,
            self.locale.as_deref(),
            outcome,
        ))
    }

    async fn emit_completed(&self, completed: ToolCompletedData) {
        if let Err(e) = self
            .emitter
            .emit(EventRequest::new(
                self.context.session_id,
                self.event_context.clone(),
                completed,
            ))
            .await
        {
            tracing::warn!(
                session_id = %self.context.session_id,
                tool_call_id = %self.tool_call.id,
                error = %e,
                "ActAtom: failed to emit tool.completed event"
            );
        }
    }

    /// Finish the call with the tool's `result`.
    pub(super) async fn finish(
        self,
        result: Result<ToolResult>,
        worker_thread_cpu_ns: Option<u64>,
    ) -> ToolCallResult {
        self.finish_as(result, worker_thread_cpu_ns, false).await
    }

    /// Settle the call; `cancelled` when a host cancelled it
    /// ([`tool_cancel`](super::tool_cancel)).
    pub(super) async fn finish_as(
        self,
        result: Result<ToolResult>,
        worker_thread_cpu_ns: Option<u64>,
        cancelled: bool,
    ) -> ToolCallResult {
        let tool_call = &self.tool_call;
        let execution_tool_call = &self.execution_tool_call;
        match result {
            Ok(mut tool_result) => {
                // Run post-tool-exec hooks (capability then final/infrastructure)
                act_hooks::run_post_tool_exec_hooks(
                    &self.post_tool_hooks,
                    &self.final_post_tool_hooks,
                    execution_tool_call,
                    &self.tool_def,
                    &mut tool_result,
                    &self.tool_context,
                )
                .await;

                let tool_duration_ms = self.tool_start.elapsed().as_millis() as u64;
                let success = tool_result.error.is_none();
                let status = if success {
                    "success"
                } else if cancelled {
                    super::tool_cancel::CANCELLED_STATUS
                } else {
                    "error"
                };

                // A delivered send_message is something the agent said. Record
                // it before the tool completes so every surface sees the
                // message no later than the call that sent it. Replays never
                // re-emit: the call settled, and so did its message.
                if success
                    && let Some(sent) = crate::conversation::sent_message(
                        &tool_call.name,
                        &tool_call.id,
                        &execution_tool_call.arguments,
                        tool_result.result.as_ref(),
                    )
                    && let Err(e) = self
                        .emitter
                        .emit(EventRequest::new(
                            self.context.session_id,
                            self.event_context.clone(),
                            sent,
                        ))
                        .await
                {
                    tracing::warn!(
                        tool_call_id = %tool_call.id,
                        error = %e,
                        "ActAtom: failed to emit conversation.message"
                    );
                }

                let phase = if success {
                    ToolNarrationPhase::Completed
                } else {
                    ToolNarrationPhase::Failed
                };
                let completed_data = ToolCompletedData::from_tool_result(
                    tool_call,
                    &tool_result,
                    status,
                    Some(tool_duration_ms),
                )
                .with_fingerprints(
                    self.tool_call_fingerprint.clone(),
                    tool_result_fingerprint(&tool_call.name, &tool_result),
                )
                .with_display_name(self.display_name.clone())
                .with_capability_attribution(
                    self.capability_id.clone(),
                    self.capability_name.clone(),
                )
                .with_executed_arguments(&self.authored_arguments, &execution_tool_call.arguments)
                .with_acted_as(client_policy::mcp_acted_as(&self.tool_context))
                .with_narration(self.narration(phase, success.then_some(&tool_result)))
                .with_worker_thread_cpu_ns(worker_thread_cpu_ns);

                tracing::debug!(
                    session_id = %self.context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    success = %success,
                    "ActAtom: tool execution completed"
                );

                // Settle the durable claim (EVE-530).
                if let Some((store, token)) = &self.claim {
                    let result_snapshot =
                        serde_json::to_value(&tool_result).unwrap_or(serde_json::Value::Null);
                    match store
                        .settle_tool_call(
                            &self.context.turn_id.to_string(),
                            &tool_call.id,
                            result_snapshot,
                            "settled",
                            *token,
                        )
                        .await
                    {
                        Ok(false) => {
                            tracing::warn!(
                                session_id = %self.context.session_id,
                                tool_call_id = %tool_call.id,
                                "ActAtom: settle ownership check failed (task reclaimed)"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                session_id = %self.context.session_id,
                                tool_call_id = %tool_call.id,
                                error = %e,
                                "ActAtom: settle_tool_call failed"
                            );
                        }
                        Ok(true) => {}
                    }
                }

                // The durable result must exist before publishing completion:
                // a replacement worker can restore it if this event never commits.
                self.emit_completed(completed_data).await;

                let conn_req = tool_result.connection_required.clone();
                ToolCallResult {
                    tool_call: tool_call.clone(),
                    result: tool_result,
                    success,
                    status: status.to_string(),
                    connection_required: conn_req,
                    determinism_fatal: None,
                }
            }
            Err(e) => {
                let tool_duration_ms = self.tool_start.elapsed().as_millis() as u64;
                let error_msg = e.to_string();

                self.emit_completed(
                    ToolCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        "error".to_string(),
                        error_msg.clone(),
                        Some(tool_duration_ms),
                    )
                    .with_worker_thread_cpu_ns(worker_thread_cpu_ns)
                    .with_fingerprints(
                        self.tool_call_fingerprint.clone(),
                        tool_error_fingerprint(&tool_call.name, "error", &error_msg),
                    )
                    .with_display_name(self.display_name.clone())
                    .with_capability_attribution(
                        self.capability_id.clone(),
                        self.capability_name.clone(),
                    )
                    .with_executed_arguments(
                        &self.authored_arguments,
                        &execution_tool_call.arguments,
                    )
                    .with_narration(self.narration(ToolNarrationPhase::Failed, None)),
                )
                .await;

                tracing::warn!(
                    session_id = %self.context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: tool execution failed"
                );

                ToolCallResult {
                    tool_call: tool_call.clone(),
                    result: ToolResult {
                        details: None,
                        tool_call_id: tool_call.id.clone(),
                        result: None,
                        images: None,
                        error: Some(error_msg),
                        connection_required: None,
                        raw_output: None,
                    },
                    success: false,
                    status: "error".to_string(),
                    connection_required: None,
                    determinism_fatal: None,
                }
            }
        }
    }
}
