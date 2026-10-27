//! Running a call its host may take over once it runs long (`tool_detach`).
//!
//! Decision: the call's tail runs as its own task from the start, so handing
//! it over moves nothing: the act stops waiting for the task and gives it to
//! the host, which owns it from then on (dropping it aborts the call). A call
//! that finishes before the threshold is awaited like an inline one and costs
//! only the spawn.

use std::sync::Arc;

use futures::future::{Either, select};
use tracing::Instrument;

use super::ToolCallResult;
use super::abort_on_drop::AbortOnDropJoinHandle;
use super::call_tail::CallTail;
use crate::engine::tool_execution::ToolExecutor;
use crate::engine::tool_types::{ToolCall, ToolResult};

use super::super::tool_detach::{self, DetachedToolCall, SwitchEmitter, ToolDetacher};

/// Run `tail` on its own task; past the host's threshold, hand it to the
/// host and report the call detached. `switches` carry the call's events
/// (its tail's and its tool's) and follow it to the host.
pub(super) async fn run<T: ToolExecutor + 'static>(
    detacher: Arc<dyn ToolDetacher>,
    switches: [Arc<SwitchEmitter>; 2],
    tail: CallTail,
    executor: Arc<T>,
    clock: Option<Arc<dyn super::super::cpu_time::ThreadCpuClock>>,
) -> ToolCallResult {
    let call = DetachedToolCall {
        session_id: tail.context.session_id,
        turn_id: tail.context.turn_id,
        input_message_id: tail.context.input_message_id,
        tool_call_id: tail.tool_call.id.clone(),
        tool_name: tail.tool_call.name.clone(),
    };
    let tool_call = tail.tool_call.clone();
    let mut task = AbortOnDropJoinHandle::new(everruns_contracts::rt::spawn(
        tail.run(executor, clock)
            .instrument(tracing::Span::current()),
    ));
    let threshold = everruns_contracts::rt::sleep(detacher.detach_after());
    futures::pin_mut!(threshold);
    if let Either::Left((joined, _)) = select(&mut task, threshold).await {
        return joined_result(joined, &tool_call);
    }
    let Some(detached) = detacher.detach(&call).await else {
        return joined_result(task.await, &tool_call);
    };
    for switch in &switches {
        switch.redirect(detached.emitter.clone());
    }
    tracing::info!(
        session_id = %call.session_id,
        turn_id = %call.turn_id,
        tool_call_id = %call.tool_call_id,
        tool_name = %call.tool_name,
        "ActAtom: tool call detached; the turn parks until it completes"
    );
    (detached.run)(Box::pin(async move {
        let _ = task.await;
    }));
    detached_result(tool_call)
}

fn joined_result(
    joined: std::result::Result<ToolCallResult, everruns_contracts::rt::JoinError>,
    tool_call: &ToolCall,
) -> ToolCallResult {
    joined.unwrap_or_else(|join_err| {
        let error = format!("tool task failed to complete: {join_err}");
        ToolCallResult {
            tool_call: tool_call.clone(),
            result: ToolResult {
                details: None,
                tool_call_id: tool_call.id.clone(),
                result: None,
                images: None,
                error: Some(error),
                connection_required: None,
                raw_output: None,
            },
            success: false,
            status: "error".to_string(),
            connection_required: None,
            determinism_fatal: None,
        }
    })
}

/// The act's placeholder for a call its host finishes: no result yet, its
/// `tool.completed` comes from the host.
fn detached_result(tool_call: ToolCall) -> ToolCallResult {
    ToolCallResult {
        result: ToolResult {
            details: None,
            tool_call_id: tool_call.id.clone(),
            result: None,
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        },
        tool_call,
        success: false,
        status: tool_detach::DETACHED_STATUS.to_string(),
        connection_required: None,
        determinism_fatal: None,
    }
}

impl<T, E> super::ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: crate::engine::event_emitter::EventEmitter + Send + Sync + 'static,
{
    /// End an act whose host took over some of its calls: the turn parks
    /// until the host recorded them. `act.completed` is not recorded, since
    /// the act has not finished; the calls that did finish here still get
    /// their post-act hooks (a connection prompt, an approval pause), whose
    /// events the resumed turn finds in the log.
    pub(super) async fn park_on_detached(
        &self,
        context: &crate::engine::ExecutionContext,
        results: Vec<ToolCallResult>,
        tool_definitions: &[crate::engine::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> crate::engine::error::Result<super::ActResult> {
        if let Some(fatal) = results.iter().find_map(|r| r.determinism_fatal.as_deref()) {
            return Err(crate::engine::error::AgentLoopError::tool(format!(
                "act activity aborted due to determinism violation: {fatal}"
            )));
        }
        let finished: Vec<_> = results
            .into_iter()
            .filter(|result| result.status != tool_detach::DETACHED_STATUS)
            .collect();
        let success_count = finished.iter().filter(|r| r.success).count() as u32;
        let error_count = finished.len() as u32 - success_count;
        let mut finished = super::ActResult {
            results: finished,
            completed: false,
            success_count,
            error_count,
            waiting_for_tool_results: false,
            waiting_for_url_elicitation: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };
        super::act_hooks::run_post_act_hooks(
            &self.hooks,
            context,
            &mut finished,
            tool_definitions,
            &self.event_emitter,
            |calls, defs| super::client_policy::request_data(self, context, calls, defs, locale),
        )
        .await;
        Err(tool_detach::parked())
    }
}
