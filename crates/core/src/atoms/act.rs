//! ActAtom - Atom for parallel tool execution
//!
//! This atom handles:
//! 1. Emitting act.started event
//! 2. Executing multiple tool calls in parallel (with tool.started/completed events)
//! 3. Handling errors, timeouts, and cancellations as "normal" results
//! 4. Emitting act.completed event
//! 5. Returning all tool results (success, error, timeout, or cancelled)
//!
//! Tool results are emitted as `tool.completed` events and returned in ActResult.
//! Messages are derived from events - no separate message storage is needed.
//!
//! Note: OTel instrumentation is handled via the event-listener pattern.
//! tool.started/completed events are emitted by this atom, and OtelEventListener
//! creates the appropriate gen-ai spans from those events.
//!
//! NOTES from Python spec:
//! - Tools call runs in parallel
//! - Error from tool call is not an error for the whole Act, error from tool is "normal" result
//! - Tool invocations should be timeouted, timeout is also "normal" result from tool
//! - Exit of act should have all tool calls finished (successfully or with error/timeout)
//! - Act and each tool call should emit start/end events
//! - Act and each tool call should be cancellable, and this is also "normal" result

use async_trait::async_trait;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use super::{Atom, AtomContext};
use crate::error::Result;
use crate::events::{
    ActCompletedData, ActStartedData, EventContext, EventRequest, ToolCompletedData,
    ToolStartedData,
};
use crate::message::ContentPart;
use crate::tool_narration::{ToolNarrationPhase, render_group_headline, render_tool_narration};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{
    AgentStore, EventEmitter, SessionFileStore, SessionMutator, SessionStore, ToolContext,
    ToolExecutor,
};
use crate::typed_id::{AgentId, HarnessId};
use uuid::Uuid;

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActInput {
    /// Organization ID for scoped data access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<i64>,
    /// Atom execution context
    pub context: AtomContext,
    /// Harness ID (needed for scheduling follow-up reason activity)
    pub harness_id: HarnessId,
    /// Agent ID (needed for scheduling follow-up reason activity, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Tool calls to execute
    pub tool_calls: Vec<ToolCall>,
    /// Available tool definitions for resolution
    pub tool_definitions: Vec<ToolDefinition>,
}

/// Result of a single tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// The original tool call
    pub tool_call: ToolCall,
    /// The result of the tool call
    pub result: ToolResult,
    /// Whether the execution was successful
    pub success: bool,
    /// Status: "success", "error", "timeout", or "cancelled"
    pub status: String,
    /// If set, the tool requires a user connection for this provider before it can execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_required: Option<String>,
}

/// Result of the ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActResult {
    /// Results for all tool calls
    pub results: Vec<ToolCallResult>,
    /// Whether all tool calls completed (regardless of success/failure)
    pub completed: bool,
    /// Number of successful tool calls
    pub success_count: u32,
    /// Number of failed tool calls
    pub error_count: u32,
    /// Providers that require user connection setup before the tool can run.
    /// When non-empty, the worker should pause and emit a client-side tool call
    /// to prompt the user to configure the connection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_required: Vec<String>,
}

// ============================================================================
// ActAtom
// ============================================================================

/// Atom that executes tool calls in parallel
///
/// This atom:
/// 1. Emits act.started event
/// 2. Executes all tool calls in parallel (emitting tool.started/completed for each)
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Emits act.completed event
/// 5. Returns comprehensive results for all tools
///
/// Tool results are emitted as events and returned in ActResult.
/// Messages are derived from events by the message store.
pub struct ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    tool_executor: T,
    event_emitter: E,
    /// Optional file store for context-aware tools
    file_store: Option<Arc<dyn SessionFileStore>>,
    /// Optional SQL database store for sql_execute/sql_query/sql_schema tools
    sqldb_store: Option<crate::traits::SessionSqlDbStoreRef>,
    /// Optional session storage store for kv_store/secret_store tools
    storage_store: Option<Arc<dyn crate::traits::SessionStorageStore>>,
    /// Optional resolver for user connection tokens
    connection_resolver: Option<Arc<dyn crate::traits::UserConnectionResolver>>,
    /// Optional session store for session metadata reads
    session_store: Option<Arc<dyn SessionStore>>,
    /// Optional session mutator for session metadata updates
    session_mutator: Option<Arc<dyn SessionMutator>>,
    /// Optional agent store for agent metadata reads
    agent_store: Option<Arc<dyn AgentStore>>,
    /// Optional session schedule store for scheduling tools
    schedule_store: Option<Arc<dyn crate::traits::SessionScheduleStore>>,
    /// Optional platform store for org-level management tools
    platform_store: Option<Arc<dyn crate::platform_store::PlatformStore>>,
    /// Optional leased resource store for provider lease tracking
    leased_resource_store: Option<Arc<dyn crate::traits::LeasedResourceStore>>,
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    /// Create a new ActAtom
    pub fn new(tool_executor: T, event_emitter: E) -> Self {
        Self {
            tool_executor,
            event_emitter,
            file_store: None,
            sqldb_store: None,
            storage_store: None,
            connection_resolver: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
        }
    }

    /// Create a new ActAtom with a file store for context-aware tools
    pub fn with_file_store(
        tool_executor: T,
        event_emitter: E,
        file_store: Arc<dyn SessionFileStore>,
    ) -> Self {
        Self {
            tool_executor,
            event_emitter,
            file_store: Some(file_store),
            sqldb_store: None,
            storage_store: None,
            connection_resolver: None,
            session_store: None,
            session_mutator: None,
            agent_store: None,
            schedule_store: None,
            platform_store: None,
            leased_resource_store: None,
        }
    }

    /// Set the SQL database store on this atom
    pub fn with_sqldb_store(mut self, store: crate::traits::SessionSqlDbStoreRef) -> Self {
        self.sqldb_store = Some(store);
        self
    }

    /// Set the session storage store on this atom
    pub fn with_storage_store(
        mut self,
        store: Arc<dyn crate::traits::SessionStorageStore>,
    ) -> Self {
        self.storage_store = Some(store);
        self
    }

    /// Set the user connection resolver on this atom
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn crate::traits::UserConnectionResolver>,
    ) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Set session store for context-aware tools.
    pub fn with_session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set session mutator for context-aware tools.
    pub fn with_session_mutator(mut self, mutator: Arc<dyn SessionMutator>) -> Self {
        self.session_mutator = Some(mutator);
        self
    }

    /// Set agent store for context-aware tools.
    pub fn with_agent_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.agent_store = Some(store);
        self
    }

    /// Set session schedule store for scheduling tools.
    pub fn with_schedule_store(
        mut self,
        store: Arc<dyn crate::traits::SessionScheduleStore>,
    ) -> Self {
        self.schedule_store = Some(store);
        self
    }

    /// Set platform store for org-level management tools.
    pub fn with_platform_store(
        mut self,
        store: Arc<dyn crate::platform_store::PlatformStore>,
    ) -> Self {
        self.platform_store = Some(store);
        self
    }

    /// Set leased resource store for lifecycle-managed provider resources.
    pub fn with_leased_resource_store(
        mut self,
        store: Arc<dyn crate::traits::LeasedResourceStore>,
    ) -> Self {
        self.leased_resource_store = Some(store);
        self
    }
}

#[async_trait]
impl<T, E> Atom for ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    type Input = ActInput;
    type Output = ActResult;

    fn name(&self) -> &'static str {
        "act"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ActInput {
            context,
            tool_calls,
            tool_definitions,
            .. // agent_id/org_id not needed here, just passed through workflow
        } = input;

        if tool_calls.is_empty() {
            return Ok(ActResult {
                results: vec![],
                completed: true,
                success_count: 0,
                error_count: 0,
                connection_required: vec![],
            });
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            tool_count = %tool_calls.len(),
            "ActAtom: executing tools in parallel"
        );

        // Generate OTel-style span IDs for hierarchical tracing
        // trace_id: groups all events in this turn
        // span_id: unique identifier for this act span (shared by started/completed)
        // parent_span_id: links to turn as parent
        //
        // NOTE: TurnId::to_string() returns prefixed format (e.g., "turn_abc123")
        // matching the format used by turn.started/completed events in Braintrust.
        let trace_id = context.turn_id.to_string();
        let act_span_id = Uuid::now_v7().to_string();
        let parent_span_id = trace_id.clone(); // Parent is the turn

        // Create event context from atom context with span info
        let event_context = EventContext::from_atom_context(&context).with_span(
            trace_id.clone(),
            act_span_id.clone(),
            Some(parent_span_id.clone()),
        );

        // Track act phase timing for Braintrust observability
        let act_start = Instant::now();

        // Build tool name to definition map
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_definitions
            .iter()
            .map(|def| {
                let name = def.name();
                (name, def)
            })
            .collect();

        // Emit act.started event (with display names from tool definitions)
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ActStartedData::with_definitions(&tool_calls, &tool_definitions),
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.started event"
            );
        }

        // Execute all tool calls in parallel (each tool event references act span as parent)
        let futures: Vec<_> = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = tool_map.get(tool_call.name.as_str()).cloned();
                self.execute_single_tool(
                    &context,
                    tool_call.clone(),
                    tool_def,
                    &trace_id,
                    &act_span_id,
                )
            })
            .collect();

        let results = join_all(futures).await;

        // Count successes and errors
        let success_count = results.iter().filter(|r| r.success).count() as u32;
        let error_count = results.iter().filter(|r| !r.success).count() as u32;

        // Collect providers that need connection setup
        let connection_required: Vec<String> = results
            .iter()
            .filter_map(|r| r.connection_required.clone())
            .collect();

        // Calculate act phase duration
        let act_duration_ms = act_start.elapsed().as_millis() as u64;

        // Emit act.completed event (same span as act.started, parent is turn)
        let completed_context = EventContext::from_atom_context(&context).with_span(
            trace_id.clone(),
            act_span_id.clone(), // Same span_id as started
            Some(parent_span_id.clone()),
        );
        let mut completed_headline = render_group_headline(
            &tool_calls,
            &tool_definitions,
            ToolNarrationPhase::Completed,
        );
        if error_count > 0 {
            let suffix = format!(
                " with {} error{}",
                error_count,
                if error_count == 1 { "" } else { "s" }
            );
            completed_headline = Some(match completed_headline {
                Some(text) => format!("{text}{suffix}"),
                None => format!("Completed tool batch{suffix}"),
            });
        }

        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                completed_context,
                ActCompletedData {
                    completed: true,
                    success_count,
                    error_count,
                    duration_ms: Some(act_duration_ms),
                    headline: completed_headline,
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.completed event"
            );
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            success_count = %success_count,
            error_count = %error_count,
            "ActAtom: all tools completed"
        );

        Ok(ActResult {
            results,
            completed: true,
            success_count,
            error_count,
            connection_required,
        })
    }
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    /// Execute a single tool call
    ///
    /// Note: OTel instrumentation is handled via event listeners.
    /// tool.started/completed events are emitted, and OtelEventListener
    /// creates gen-ai spans from those events.
    async fn execute_single_tool(
        &self,
        context: &AtomContext,
        tool_call: ToolCall,
        tool_def: Option<&ToolDefinition>,
        trace_id: &str,
        act_span_id: &str,
    ) -> ToolCallResult {
        tracing::debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            tool_name = %tool_call.name,
            tool_call_id = %tool_call.id,
            "ActAtom: executing tool"
        );

        // Generate a unique span_id for this tool call (child of act span)
        let tool_span_id = Uuid::now_v7().to_string();

        // Create event context from atom context (with act span as parent)
        let event_context = EventContext::from_atom_context(context).with_span(
            trace_id.to_string(),
            tool_span_id.clone(),
            Some(act_span_id.to_string()),
        );

        // Track tool call timing for Braintrust observability
        let tool_start = Instant::now();

        // Resolve display name from tool definition
        let display_name = tool_def.and_then(|d| d.display_name().map(|s| s.to_string()));

        // Emit tool.started event (child of act.started)
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ToolStartedData {
                    tool_call: tool_call.clone(),
                    display_name: display_name.clone(),
                    narration: Some(render_tool_narration(
                        tool_def,
                        &tool_call,
                        ToolNarrationPhase::Started,
                    )),
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                tool_call_id = %tool_call.id,
                error = %e,
                "ActAtom: failed to emit tool.started event"
            );
        }

        // If tool definition not found, return error result
        let Some(tool_def) = tool_def else {
            let error_msg = format!("Tool definition not found: {}", tool_call.name);
            let tool_duration_ms = tool_start.elapsed().as_millis() as u64;

            // Emit tool.completed event for error (child of act.started)
            if let Err(e) = self
                .event_emitter
                .emit(EventRequest::new(
                    context.session_id,
                    event_context,
                    ToolCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        "error".to_string(),
                        error_msg.clone(),
                        Some(tool_duration_ms),
                    )
                    .with_narration(Some(render_tool_narration(
                        None,
                        &tool_call,
                        ToolNarrationPhase::Failed,
                    ))),
                ))
                .await
            {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: failed to emit tool.completed event"
                );
            }

            return ToolCallResult {
                tool_call: tool_call.clone(),
                result: ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: None,
                    images: None,
                    error: Some(error_msg),
                    connection_required: None,
                },
                success: false,
                status: "error".to_string(),
                connection_required: None,
            };
        };

        // Execute the tool
        let result = if self.file_store.is_some()
            || self.sqldb_store.is_some()
            || self.storage_store.is_some()
            || self.connection_resolver.is_some()
            || self.session_store.is_some()
            || self.session_mutator.is_some()
            || self.agent_store.is_some()
            || self.schedule_store.is_some()
            || self.platform_store.is_some()
            || self.leased_resource_store.is_some()
        {
            let mut tool_context = if let Some(ref store) = self.file_store {
                ToolContext::with_file_store(context.session_id, store.clone())
            } else {
                ToolContext::new(context.session_id)
            };
            if let Some(ref store) = self.sqldb_store {
                tool_context.sqldb_store = Some(store.clone());
            }
            if let Some(ref store) = self.storage_store {
                tool_context.storage_store = Some(store.clone());
            }
            if let Some(ref resolver) = self.connection_resolver {
                tool_context.connection_resolver = Some(resolver.clone());
            }
            if let Some(ref store) = self.session_store {
                tool_context.session_store = Some(store.clone());
            }
            if let Some(ref mutator) = self.session_mutator {
                tool_context.session_mutator = Some(mutator.clone());
            }
            if let Some(ref store) = self.agent_store {
                tool_context.agent_store = Some(store.clone());
            }
            if let Some(ref store) = self.schedule_store {
                tool_context.schedule_store = Some(store.clone());
            }
            if let Some(ref store) = self.platform_store {
                tool_context.platform_store = Some(store.clone());
            }
            if let Some(ref store) = self.leased_resource_store {
                tool_context.leased_resource_store = Some(store.clone());
            }
            self.tool_executor
                .execute_with_context(&tool_call, tool_def, &tool_context)
                .await
        } else {
            self.tool_executor.execute(&tool_call, tool_def).await
        };

        match result {
            Ok(tool_result) => {
                let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
                let success = tool_result.error.is_none();
                let status = if success { "success" } else { "error" };

                // Emit tool.completed event
                let completed_data = if success {
                    // Convert result to ContentPart (text + optional images)
                    let mut result_content = tool_result
                        .result
                        .as_ref()
                        .map(|r| vec![ContentPart::text(r.to_string())])
                        .unwrap_or_default();
                    // Append images as native Image content parts
                    if let Some(ref images) = tool_result.images {
                        for img in images {
                            result_content.push(ContentPart::Image(
                                crate::message::ImageContentPart::from_base64(
                                    &img.base64,
                                    &img.media_type,
                                ),
                            ));
                        }
                    }
                    ToolCompletedData::success(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        result_content,
                        Some(tool_duration_ms),
                    )
                    .with_display_name(display_name.clone())
                    .with_narration(Some(render_tool_narration(
                        Some(tool_def),
                        &tool_call,
                        ToolNarrationPhase::Completed,
                    )))
                } else {
                    ToolCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        status.to_string(),
                        tool_result.error.clone().unwrap_or_default(),
                        Some(tool_duration_ms),
                    )
                    .with_display_name(display_name.clone())
                    .with_narration(Some(render_tool_narration(
                        Some(tool_def),
                        &tool_call,
                        ToolNarrationPhase::Failed,
                    )))
                };

                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context.clone(),
                        completed_data,
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %e,
                        "ActAtom: failed to emit tool.completed event"
                    );
                }

                tracing::debug!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    success = %success,
                    "ActAtom: tool execution completed"
                );

                let conn_req = tool_result.connection_required.clone();
                ToolCallResult {
                    tool_call,
                    result: tool_result,
                    success,
                    status: status.to_string(),
                    connection_required: conn_req,
                }
            }
            Err(e) => {
                let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
                let error_msg = e.to_string();

                // Emit tool.completed event for error
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context,
                        ToolCompletedData::failure(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            "error".to_string(),
                            error_msg.clone(),
                            Some(tool_duration_ms),
                        )
                        .with_display_name(display_name.clone())
                        .with_narration(Some(render_tool_narration(
                            Some(tool_def),
                            &tool_call,
                            ToolNarrationPhase::Failed,
                        ))),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %emit_err,
                        "ActAtom: failed to emit tool.completed event"
                    );
                }

                tracing::warn!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: tool execution failed"
                );

                ToolCallResult {
                    tool_call: tool_call.clone(),
                    result: ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        result: None,
                        images: None,
                        error: Some(error_msg),
                        connection_required: None,
                    },
                    success: false,
                    status: "error".to_string(),
                    connection_required: None,
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use crate::traits::NoopEventEmitter;
    use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use serde_json::json;

    #[tokio::test]
    async fn test_act_atom_empty_tool_calls() {
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![],
            tool_definitions: vec![],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert!(result.results.is_empty());
        assert_eq!(result.success_count, 0);
        assert_eq!(result.error_count, 0);
    }

    #[tokio::test]
    async fn test_act_atom_tool_not_found() {
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "nonexistent_tool".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        assert!(!result.results[0].success);
        assert_eq!(result.results[0].status, "error");
        assert!(
            result.results[0]
                .result
                .error
                .as_ref()
                .unwrap()
                .contains("not found")
        );
    }

    fn manage_harnesses_tool_def() -> crate::ToolDefinition {
        crate::ToolDefinition::Builtin(crate::BuiltinTool {
            name: "manage_harnesses".to_string(),
            display_name: None,
            description: "CRUD operations for harnesses".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operation": {"type": "string"}
                },
                "required": ["operation"]
            }),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
        })
    }

    #[tokio::test]
    async fn test_act_atom_platform_tool_works_with_platform_store() {
        use crate::capabilities::{Capability, PlatformManagementCapability};

        let mut executor = ToolRegistry::with_defaults();
        for tool in PlatformManagementCapability.tools() {
            executor.register_boxed(tool);
        }
        let event_emitter = NoopEventEmitter;

        // Build mock platform store
        let mock_store = crate::platform_store::tests::MockPlatformStore::new();
        let platform_store: Arc<dyn crate::platform_store::PlatformStore> = Arc::new(mock_store);

        let atom = ActAtom::new(executor, event_emitter).with_platform_store(platform_store);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "manage_harnesses".to_string(),
                arguments: json!({"operation": "list"}),
            }],
            tool_definitions: vec![manage_harnesses_tool_def()],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        assert!(
            result.results[0].success,
            "manage_harnesses should succeed when platform_store is wired: {:?}",
            result.results[0].result.error
        );
    }

    /// Regression test: `manage_harnesses` called through ActAtom WITHOUT
    /// `platform_store` should produce an error message in the result body.
    /// ToolErrors are "normal" results (success=true) by design, but the
    /// result body contains `{"error": "..."}` that the LLM sees.
    #[tokio::test]
    async fn test_act_atom_platform_tool_fails_without_platform_store() {
        use crate::capabilities::{Capability, PlatformManagementCapability};

        let mut executor = ToolRegistry::with_defaults();
        for tool in PlatformManagementCapability.tools() {
            executor.register_boxed(tool);
        }
        let event_emitter = NoopEventEmitter;

        // No stores set → ActAtom uses plain `execute()` which the tool
        // returns a ToolError from ("requires context").
        let atom = ActAtom::new(executor, event_emitter);

        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let input = ActInput {
            org_id: Some(1),
            context,
            harness_id: HarnessId::from_seed(1),
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "manage_harnesses".to_string(),
                arguments: json!({"operation": "list"}),
            }],
            tool_definitions: vec![manage_harnesses_tool_def()],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        // ToolErrors produce result body with {"error": "..."} but the ActAtom
        // treats them as successful completions (error field is None).
        let result_body = result.results[0].result.result.as_ref().unwrap();
        let err_msg = result_body["error"].as_str().unwrap();
        assert!(
            err_msg.contains("requires context"),
            "Expected 'requires context' error, got: {err_msg}"
        );
    }

    #[test]
    fn test_act_result_connection_required_serialization() {
        let result = ActResult {
            results: vec![ToolCallResult {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "daytona_create_sandbox".to_string(),
                    arguments: json!({}),
                },
                result: ToolResult {
                    tool_call_id: "call_1".to_string(),
                    result: Some(json!({"connection_required": "daytona"})),
                    images: None,
                    error: None,
                    connection_required: Some("daytona".to_string()),
                },
                success: false,
                status: "success".to_string(),
                connection_required: Some("daytona".to_string()),
            }],
            completed: true,
            success_count: 0,
            error_count: 0,
            connection_required: vec!["daytona".to_string()],
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let parsed: ActResult = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.connection_required, vec!["daytona".to_string()]);
        assert_eq!(
            parsed.results[0].connection_required,
            Some("daytona".to_string())
        );
    }

    #[test]
    fn test_act_result_without_connection_required_backward_compat() {
        // Old JSON without connection_required fields still deserializes
        let json_str = r#"{
            "results": [],
            "completed": true,
            "success_count": 0,
            "error_count": 0
        }"#;
        let parsed: ActResult = serde_json::from_str(json_str).unwrap();

        assert!(parsed.connection_required.is_empty());
    }
}
