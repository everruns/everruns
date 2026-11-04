//! Host-injected CPU observation stays on the task invoking the executor.
use super::abort_on_drop::AbortOnDropJoinHandle;
use super::{
    ActAtom, Arc, PhaseEffectSink, Result, ToolCall, ToolDefinition, ToolExecutor, ToolResult,
};
use crate::engine::tool_context::ToolContext;

impl<T: ToolExecutor, E: PhaseEffectSink> ActAtom<T, E> {
    /// Observe actual thread CPU while polling an invoked executor future.
    /// The host supplies the clock; portable/custom hosts may leave it absent.
    pub fn with_thread_cpu_clock(
        mut self,
        clock: Option<Arc<dyn crate::engine::ThreadCpuClock>>,
    ) -> Self {
        self.thread_cpu_clock = clock;
        self
    }
}

impl<T: ToolExecutor + 'static, E: PhaseEffectSink> ActAtom<T, E> {
    pub(super) async fn invoke_observed(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> (Result<ToolResult>, Option<u64>) {
        if tool_def.is_cpu_bound() {
            // CPU-bound / non-yielding in-process tools (e.g. the bash
            // interpreter) get their own task so a long synchronous burst
            // cannot starve the cooperative polling of I/O-bound tools running
            // alongside them in this act batch. On the multi-thread runtime the
            // spawned task can also progress on another worker thread.
            let executor = self.tool_executor.clone();
            let call = tool_call.clone();
            let def = tool_def.clone();
            let ctx = context.clone();
            let clock = self.thread_cpu_clock.clone();
            match AbortOnDropJoinHandle::new(everruns_contracts::rt::spawn(async move {
                super::super::cpu_time::observe_cpu(
                    executor.execute_with_context(&call, &def, &ctx),
                    clock.as_deref(),
                )
                .await
            }))
            .await
            {
                Ok(result) => result,
                Err(join_err) => (
                    Err(crate::engine::error::AgentLoopError::tool(format!(
                        "tool task failed to complete: {join_err}"
                    ))),
                    None,
                ),
            }
        } else {
            super::super::cpu_time::observe_cpu(
                self.tool_executor
                    .execute_with_context(tool_call, tool_def, context),
                self.thread_cpu_clock.as_deref(),
            )
            .await
        }
    }
}
