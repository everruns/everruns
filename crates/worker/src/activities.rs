// Workflow step implementations
//
// Steps are the units of work scheduled by the durable workflow orchestrator.
// Each step runs as a task and returns a result.
// Decision: Workers communicate with control-plane via gRPC for all operations.
//
// These implementations use Atoms from everruns-core for the actual work:
// - InputAtom: Retrieves user input message
// - ReasonAtom: LLM call with context preparation
// - ActAtom: Parallel tool execution
//
// Atoms handle message loading internally via MessageRetriever trait.
// Atoms emit events via EventEmitter for observability.

use anyhow::{Context, Result};
use everruns_core::ToolRegistry;
use everruns_core::atoms::{ActAtom, Atom, InputAtom, ReasonAtom};
use everruns_core::capabilities::{SystemPromptContext, collect_capabilities, is_mcp_capability};
use everruns_core::traits::AgentStore;
use everruns_core::{Message, PlatformDefinition};
use std::sync::Arc;

use crate::grpc_adapters::{
    GrpcAgentStore, GrpcClient, GrpcConnectionResolver, GrpcEventEmitter, GrpcHarnessStore,
    GrpcImageResolver, GrpcLeasedResourceStore, GrpcLlmProviderStore, GrpcMessageRetriever,
    GrpcPlatformStore, GrpcScheduleStore, GrpcSessionFileStore, GrpcSessionMutator,
    GrpcSessionSqlDbStore, GrpcSessionStorageStore, GrpcSessionStore,
};

// Re-export atom types for activity callers
pub use everruns_core::atoms::{
    ActInput, ActResult, InputAtomInput, InputAtomResult, ReasonInput, ReasonResult, ToolCallResult,
};

/// Thin wrapper: detect dependency blockers using core's shared logic with gRPC adapters.
async fn detect_dependency_blocker(
    grpc_client: &GrpcClient,
    org_id: i64,
    harness_id: everruns_core::HarnessId,
    agent_id: Option<everruns_core::AgentId>,
) -> Result<Option<everruns_core::DependencyBlocker>> {
    let harness_store = GrpcHarnessStore::new(grpc_client.clone(), org_id);
    let agent_store = GrpcAgentStore::new(grpc_client.clone(), org_id);
    everruns_core::detect_dependency_blocker(&harness_store, &agent_store, harness_id, agent_id)
        .await
        .map_err(anyhow::Error::new)
}

/// Emit dependency-blocked events (output message + turn.failed + session.idled, set idle).
async fn emit_dependency_blocked_events(
    grpc_client: &GrpcClient,
    org_id: i64,
    context: &everruns_core::atoms::AtomContext,
    blocker: everruns_core::DependencyBlocker,
) {
    use everruns_core::events::{
        EventContext, EventRequest, OutputMessageCompletedData, SessionIdledData, TurnFailedData,
    };
    use everruns_core::traits::EventEmitter;

    let session_id = context.session_id;
    let turn_id = context.turn_id;
    let input_message_id = context.input_message_id;
    let message = blocker.message();
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

    let output_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        OutputMessageCompletedData::new(Message::assistant(message)),
    );
    if let Err(e) = event_emitter.emit(output_event).await {
        tracing::warn!(error = %e, "Failed to emit dependency blocked message");
    }

    if let Err(e) = grpc_client
        .set_session_status(org_id, session_id, "idle")
        .await
    {
        tracing::warn!(error = %e, "Failed to set session status to idle");
    }

    let failed_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        TurnFailedData {
            turn_id,
            error: message.to_string(),
            error_code: Some(blocker.error_code().to_string()),
        },
    );
    if let Err(e) = event_emitter.emit(failed_event).await {
        tracing::warn!(error = %e, "Failed to emit dependency turn.failed");
    }

    let idled_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        SessionIdledData {
            turn_id,
            iterations: None,
            usage: None,
        },
    );
    if let Err(e) = event_emitter.emit(idled_event).await {
        tracing::warn!(error = %e, "Failed to emit dependency session.idled");
    }
}

// ============================================================================
// Activity Implementations
// ============================================================================

/// Process user input using InputAtom
///
/// This activity:
/// 1. Sets session status to "active" and emits session.activated event
/// 2. Emits turn.started event
/// 3. Retrieves the user message from the message store
/// 4. Returns the message for downstream processing
pub async fn input_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: InputAtomInput,
) -> Result<InputAtomResult> {
    use everruns_core::MessageRetriever;
    use everruns_core::events::{
        EventContext, EventRequest, SessionActivatedData, TurnStartedData,
    };
    use everruns_core::traits::EventEmitter;

    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        input_message_id = %input.context.input_message_id,
        "Executing input_activity"
    );

    // Set session status to "active" - turn is starting
    if let Err(e) = grpc_client
        .set_session_status(org_id, input.context.session_id, "active")
        .await
    {
        tracing::warn!(error = %e, "Failed to set session status to active");
    }

    // Create message retriever early to fetch input content for observability
    let message_retriever = GrpcMessageRetriever::new(grpc_client.clone());

    // Fetch input message content for turn.started event (for Braintrust observability)
    let input_content = match message_retriever
        .get(input.context.session_id, input.context.input_message_id)
        .await
    {
        Ok(Some(msg)) => Some(msg.content_to_llm_string()),
        Ok(None) => {
            tracing::warn!("Input message not found for turn.started event");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to fetch input message for turn.started event");
            None
        }
    };

    // Emit session.activated event
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());
    let activated_event = EventRequest::new(
        input.context.session_id,
        EventContext::turn(input.context.turn_id, input.context.input_message_id),
        SessionActivatedData {
            turn_id: input.context.turn_id,
            input_message_id: input.context.input_message_id,
        },
    );
    if let Err(e) = event_emitter.emit(activated_event).await {
        tracing::warn!(error = %e, "Failed to emit session.activated event");
    }

    // Emit turn.started event with input content for observability
    let turn_started_event = EventRequest::new(
        input.context.session_id,
        EventContext::turn(input.context.turn_id, input.context.input_message_id),
        TurnStartedData {
            turn_id: input.context.turn_id,
            input_message_id: input.context.input_message_id,
            input_content,
        },
    );
    if let Err(e) = event_emitter.emit(turn_started_event).await {
        tracing::warn!(error = %e, "Failed to emit turn.started event");
    }

    let atom = InputAtom::new(message_retriever);

    atom.execute(input)
        .await
        .context("InputAtom execution failed")
}

/// Call the LLM model for reasoning using ReasonAtom
///
/// This activity:
/// 1. Emits reason.started event
/// 2. Retrieves agent and session configuration
/// 3. Loads messages and prepares context
/// 4. Calls the LLM with the messages
/// 5. Stores the assistant response
/// 6. Emits reason.completed event
/// 7. Returns the result with tool calls (if any)
/// 8. If turn completes (no tool calls), emits turn.completed, sets session status to "idle" and emits session.idled
///
/// Note: API key decryption is handled by the control-plane gRPC service.
pub async fn reason_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: ReasonInput,
    platform_definition: &PlatformDefinition,
) -> Result<ReasonResult> {
    use everruns_core::MessageRetriever;
    use everruns_core::events::{
        EventContext, EventRequest, SessionIdledData, TurnCompletedData, TurnFailedData,
    };
    use everruns_core::traits::EventEmitter;

    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        agent_id = ?input.agent_id,
        "Executing reason_activity"
    );

    let session_id = input.context.session_id;
    let turn_id = input.context.turn_id;
    let input_message_id = input.context.input_message_id;
    let iteration = input.iteration;

    if let Some(blocker) =
        detect_dependency_blocker(&grpc_client, org_id, input.harness_id, input.agent_id).await?
    {
        emit_dependency_blocked_events(&grpc_client, org_id, &input.context, blocker).await;
        return Ok(ReasonResult {
            success: false,
            text: blocker.message().to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: 100,
            error: Some("dependency_unavailable".to_string()),
            usage: None,
            response_id: None,
            locale: None,
        });
    }

    // Create atom dependencies using gRPC adapters
    let harness_store = GrpcHarnessStore::new(grpc_client.clone(), org_id);
    let agent_store = GrpcAgentStore::new(grpc_client.clone(), org_id);
    let session_store = GrpcSessionStore::new(grpc_client.clone(), org_id);
    let message_retriever = GrpcMessageRetriever::new(grpc_client.clone());
    let provider_store = GrpcLlmProviderStore::new(grpc_client.clone(), org_id);
    let capability_registry = platform_definition.capability_registry().clone();
    let driver_registry = platform_definition.driver_registry().clone();
    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

    // Create image resolver for multimodal image support
    let image_resolver = Arc::new(GrpcImageResolver::new(grpc_client.clone(), org_id));

    // Create file store for AGENTS.md reading (agent_instructions capability)
    let file_store = Arc::new(GrpcSessionFileStore::new(grpc_client.clone()));

    let atom = ReasonAtom::new(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        event_emitter,
    )
    .with_image_resolver(image_resolver)
    .with_file_store(file_store);

    let result = atom
        .execute(input)
        .await
        .context("ReasonAtom execution failed")?;

    // If turn is complete (no tool calls, or failure), set session to idle
    let turn_complete = !result.has_tool_calls || !result.success;
    if turn_complete {
        // Set session status to "idle"
        if let Err(e) = grpc_client
            .set_session_status(org_id, session_id, "idle")
            .await
        {
            tracing::warn!(error = %e, "Failed to set session status to idle");
        }

        let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

        // Emit turn.failed or turn.completed based on success
        if !result.success {
            // Emit turn.failed event with sanitized error message
            let turn_failed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnFailedData {
                    turn_id,
                    error: "An error occurred while processing your request.".to_string(),
                    error_code: Some("llm_error".to_string()),
                },
            );
            if let Err(e) = event_emitter.emit(turn_failed_event).await {
                tracing::warn!(error = %e, "Failed to emit turn.failed event");
            }
        } else {
            // Fetch input message content for turn.completed (for Braintrust observability)
            let message_retriever = GrpcMessageRetriever::new(grpc_client.clone());
            let input_content = match message_retriever.get(session_id, input_message_id).await {
                Ok(Some(msg)) => Some(msg.content_to_llm_string()),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to fetch input message for turn.completed");
                    None
                }
            };

            // Emit turn.completed event with usage
            let turn_completed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnCompletedData {
                    turn_id,
                    iterations: iteration,
                    duration_ms: None,
                    usage: result.usage.clone(),
                    input_content,
                },
            );
            if let Err(e) = event_emitter.emit(turn_completed_event).await {
                tracing::warn!(error = %e, "Failed to emit turn.completed event");
            }
        }

        // Emit session.idled event
        // Note: Using turn usage as fallback since worker doesn't have DB access
        // for cumulative session usage. The UI should handle accumulation.
        let idled_event = EventRequest::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: Some(iteration),
                usage: result.usage.clone(),
            },
        );
        if let Err(e) = event_emitter.emit(idled_event).await {
            tracing::warn!(error = %e, "Failed to emit session.idled event");
        }
    }

    Ok(result)
}

/// Execute tools in parallel using ActAtom
///
/// This activity:
/// 1. Emits act.started event
/// 2. Executes all tool calls in parallel (emitting tool.started/completed for each)
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Stores tool result messages
/// 5. Emits act.completed event
/// 6. Returns comprehensive results for all tools
///
/// Supports both built-in tools and MCP tools (via remote MCP servers).
pub async fn act_activity(
    grpc_client: GrpcClient,
    org_id: i64,
    input: ActInput,
    platform_definition: &PlatformDefinition,
) -> Result<ActResult> {
    tracing::info!(
        org_id = org_id,
        session_id = %input.context.session_id,
        turn_id = %input.context.turn_id,
        tool_count = %input.tool_calls.len(),
        "Executing act_activity"
    );

    if let Some(blocker) =
        detect_dependency_blocker(&grpc_client, org_id, input.harness_id, input.agent_id).await?
    {
        emit_dependency_blocked_events(&grpc_client, org_id, &input.context, blocker).await;
        return Ok(ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 1,
            waiting_for_tool_results: false,
            blocked: true,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        });
    }

    // Create tool registry with default built-in tools
    let mut builtin_executor = ToolRegistry::with_defaults();

    // Resolve blueprint_id: prefer ActInput field, fall back to session lookup
    let blueprint_id = if input.blueprint_id.is_some() {
        input.blueprint_id.clone()
    } else {
        let session_store = GrpcSessionStore::new(grpc_client.clone(), org_id);
        match session_store.get_session(input.context.session_id).await {
            Ok(Some(session)) => session.blueprint_id.clone(),
            _ => None,
        }
    };

    // Load tools: blueprint tools for blueprint sessions, capability tools otherwise.
    if let Some(ref blueprint_id) = blueprint_id {
        // Blueprint path: load tools from the blueprint definition
        let capability_registry = platform_definition.capability_registry().clone();
        if let Some(blueprint) = capability_registry.blueprint(blueprint_id) {
            for tool in blueprint.tools {
                builtin_executor.register_boxed(tool);
            }
            tracing::debug!(
                blueprint_id = blueprint_id,
                "Registered blueprint tools for act_activity"
            );
        } else {
            tracing::warn!(
                blueprint_id = blueprint_id,
                "Blueprint not found in registry for act_activity"
            );
        }
    } else {
        // Standard path: load capabilities from agent or harness.
        // When agent_id is present, use agent capabilities.
        // When agent_id is absent, fall back to harness capabilities so that
        // harness-provided tools (e.g. bash) are still registered.
        let cap_ids: Vec<String> = if let Some(agent_id) = input.agent_id {
            let agent_store = GrpcAgentStore::new(grpc_client.clone(), org_id);
            if let Ok(Some(agent)) = agent_store.get_agent(agent_id).await {
                agent
                    .capabilities
                    .iter()
                    .map(|c| c.capability_id().to_string())
                    .filter(|id| !is_mcp_capability(id))
                    .collect()
            } else {
                vec![]
            }
        } else {
            let harness_store = GrpcHarnessStore::new(grpc_client.clone(), org_id);
            if let Ok(Some(harness)) =
                everruns_core::traits::HarnessStore::get_harness(&harness_store, input.harness_id)
                    .await
            {
                harness
                    .capabilities
                    .iter()
                    .map(|c| c.capability_id().to_string())
                    .filter(|id| !is_mcp_capability(id))
                    .collect()
            } else {
                vec![]
            }
        };

        if !cap_ids.is_empty() {
            let capability_registry = platform_definition.capability_registry().clone();
            let ctx = SystemPromptContext::without_file_store(input.context.session_id);
            let collected = collect_capabilities(&cap_ids, &capability_registry, &ctx).await;
            for tool in collected.tools {
                builtin_executor.register_boxed(tool);
            }
            tracing::debug!(
                capability_count = cap_ids.len(),
                tool_count = collected.tool_definitions.len(),
                "Registered capability tools for act_activity"
            );
        }
    }

    // Create composite tool executor that handles both built-in and MCP tools
    let mcp_executor = Arc::new(crate::mcp_executor::McpToolExecutor::new(
        grpc_client.clone(),
        org_id,
    ));
    let tool_executor =
        crate::mcp_executor::CompositeToolExecutor::new(builtin_executor, mcp_executor);

    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());
    let file_store = Arc::new(GrpcSessionFileStore::new(grpc_client.clone()));
    let storage_store: Arc<dyn everruns_core::traits::SessionStorageStore> =
        Arc::new(GrpcSessionStorageStore::new(grpc_client.clone()));
    let connection_resolver: Arc<dyn everruns_core::traits::UserConnectionResolver> =
        Arc::new(GrpcConnectionResolver::new(grpc_client.clone()));
    let session_store: Arc<dyn everruns_core::traits::SessionStore> =
        Arc::new(GrpcSessionStore::new(grpc_client.clone(), org_id));
    let session_mutator: Arc<dyn everruns_core::traits::SessionMutator> =
        Arc::new(GrpcSessionMutator::new(grpc_client.clone(), org_id));
    let agent_store: Arc<dyn everruns_core::traits::AgentStore> =
        Arc::new(GrpcAgentStore::new(grpc_client.clone(), org_id));
    let sqldb_store: everruns_core::traits::SessionSqlDbStoreRef =
        Arc::new(GrpcSessionSqlDbStore::new(grpc_client.clone()));
    let leased_resource_store: Arc<dyn everruns_core::traits::LeasedResourceStore> =
        Arc::new(GrpcLeasedResourceStore::new(grpc_client.clone()));
    let schedule_store: Arc<dyn everruns_core::traits::SessionScheduleStore> =
        Arc::new(GrpcScheduleStore::new(grpc_client.clone(), org_id));
    let platform_store: Arc<dyn everruns_core::platform_store::PlatformStore> =
        Arc::new(GrpcPlatformStore::new(grpc_client, org_id));

    let atom = ActAtom::with_file_store(tool_executor, event_emitter, file_store)
        .with_storage_store(storage_store)
        .with_connection_resolver(connection_resolver)
        .with_session_store(session_store)
        .with_session_mutator(session_mutator)
        .with_agent_store(agent_store)
        .with_sqldb_store(sqldb_store)
        .with_leased_resource_store(leased_resource_store)
        .with_schedule_store(schedule_store)
        .with_platform_store(platform_store)
        .with_capability_registry(platform_definition.capability_registry().clone());

    atom.execute(input)
        .await
        .context("ActAtom execution failed")
}

// ============================================================================
// Activity Type Constants
// ============================================================================

/// Activity type constants for workflow scheduling
pub mod activity_types {
    pub const INPUT: &str = "input";
    pub const REASON: &str = "reason";
    pub const ACT: &str = "act";
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::atoms::AtomContext;
    use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
    use everruns_core::{BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolPolicy};
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn test_input_atom_input_serialization() {
        let context = AtomContext::new(
            SessionId::from_uuid(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()),
            TurnId::from_uuid(Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap()),
            MessageId::from_uuid(Uuid::parse_str("770e8400-e29b-41d4-a716-446655440000").unwrap()),
        );

        let input = InputAtomInput { context };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: InputAtomInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.context.session_id.uuid(),
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_reason_input_serialization() {
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        let input = ReasonInput {
            context: context.clone(),
            agent_id: Some(AgentId::from_uuid(
                Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap(),
            )),
            harness_id: HarnessId::from_uuid(
                Uuid::parse_str("990e8400-e29b-41d4-a716-446655440000").unwrap(),
            ),
            org_id: 1,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ReasonInput = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.agent_id.unwrap().uuid(),
            Uuid::parse_str("880e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn test_act_input_serialization() {
        let context = AtomContext::new(SessionId::new(), TurnId::new(), MessageId::new());

        let input = ActInput {
            org_id: Some(1),
            context: context.clone(),
            agent_id: Some(AgentId::new()),
            harness_id: HarnessId::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "NYC"}),
            }],
            tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
                name: "get_weather".to_string(),
                display_name: None,
                description: "Get weather".to_string(),
                parameters: json!({}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
            })],
            locale: None,
            blueprint_id: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: ActInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "get_weather");
    }

    #[test]
    fn test_reason_result_serialization() {
        let result = ReasonResult {
            success: true,
            text: "Hello!".to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: 10,
            error: None,
            usage: None,
            response_id: None,
            locale: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ReasonResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.text, "Hello!");
    }

    #[test]
    fn test_act_result_serialization() {
        let result = ActResult {
            results: vec![ToolCallResult {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    arguments: json!({}),
                },
                result: everruns_core::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    result: Some(json!({"temp": 72})),
                    images: None,
                    error: None,
                    connection_required: None,
                },
                success: true,
                status: "success".to_string(),
                connection_required: None,
            }],
            completed: true,
            success_count: 1,
            error_count: 0,
            waiting_for_tool_results: false,
            blocked: false,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ActResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.completed);
        assert_eq!(parsed.success_count, 1);
    }
}
