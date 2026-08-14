//! A downstream-style pure-kernel fixture: project values, assemble from
//! already-resolved inputs, and execute without implementing platform stores.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use async_trait::async_trait;
use everruns_core::ExecutionContext;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::{
    AssembledTurnContext, CapabilityRegistry, ExecutionSession, HarnessDefinition, Message,
    ResolvedExecutionSnapshot, ResolvedModelExecution, ResolvedTurnContextInput,
    TurnContextRequest, TurnContextResolver, assemble_resolved_turn_context,
};
use everruns_engine::{ReasonAtom, ReasonInput};
use everruns_provider::driver_registry::{
    ChatDriver, LlmCallConfig, LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::provider::DriverId;
use everruns_provider::runtime_provider::{ProviderEndpoint, ProviderKey};
use everruns_provider::typed_id::{MessageId, ModelId, SessionId, TurnId, WorkspaceId};
use futures::stream;

struct FixedDriver;

#[async_trait]
impl ChatDriver for FixedDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<everruns_provider::driver_registry::LlmMessage>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        Ok(Box::pin(stream::iter([
            Ok(LlmStreamEvent::TextDelta("resolved input works".into())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

#[derive(Clone)]
struct FixedHistory(Vec<Message>);

#[async_trait]
impl MessageRetriever for FixedHistory {
    async fn get(&self, _session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        Ok(self
            .0
            .iter()
            .find(|message| message.id == message_id)
            .cloned())
    }

    async fn load(&self, _session_id: SessionId) -> Result<Vec<Message>> {
        Ok(self.0.clone())
    }
}

struct NoStoreResolver;

#[async_trait]
impl TurnContextResolver for NoStoreResolver {
    async fn resolve_turn_context(
        &self,
        _request: TurnContextRequest,
    ) -> Result<AssembledTurnContext> {
        Err(AgentLoopError::config(
            "preassembled fixture must never resolve stores",
        ))
    }
}

struct RecordingEmitter(AtomicI32);

#[async_trait]
impl EventEmitter for RecordingEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let sequence = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(request.into_event(everruns_provider::typed_id::EventId::new(), sequence))
    }
}

#[tokio::test]
async fn kernel_executes_from_resolved_values_without_stores() {
    let harness_id = everruns_provider::typed_id::HarnessId::from_seed(905);
    let session_id = SessionId::from_seed(905);
    let workspace_id = WorkspaceId::from_seed(905);
    let harness = HarnessDefinition::new("pure-kernel", "Answer directly.");
    let session = ExecutionSession::new(session_id, workspace_id, harness_id);
    let snapshot = ResolvedExecutionSnapshot::project(&harness, None, &session).unwrap();
    let user = Message::user("Does the resolved path work?");
    let messages = vec![user.clone()];
    let capability_registry = CapabilityRegistry::new();
    let model_id = ModelId::from_seed(905);
    let assembled = assemble_resolved_turn_context(
        ResolvedTurnContextInput {
            snapshot,
            messages: messages.clone(),
            message_source_sequence: None,
            model: ResolvedModelExecution {
                model: "external-model".into(),
                provider: ProviderKey::new("external-provider"),
                provider_type: DriverId::LlmSim,
                driver: Arc::new(FixedDriver),
            },
            resolved_model_id: Some(model_id),
            mcp_tool_definitions: vec![],
        },
        &capability_registry,
        None,
    )
    .await
    .unwrap();

    let atom = ReasonAtom::new(
        NoStoreResolver,
        FixedHistory(messages),
        capability_registry,
        RecordingEmitter(AtomicI32::new(0)),
    );
    let result = atom
        .execute_with_assembled_context(
            ReasonInput {
                context: ExecutionContext::new(session_id, TurnId::new(), user.id)
                    .with_workspace_id(workspace_id),
                harness_id,
                agent_id: None,
                org_id: 0,
                mcp_tool_definitions: vec![],
                previous_response_id: None,
                iteration: 1,
            },
            assembled,
        )
        .await
        .unwrap();

    assert!(result.success, "{result:?}");
    assert_eq!(result.text, "resolved input works");
}
