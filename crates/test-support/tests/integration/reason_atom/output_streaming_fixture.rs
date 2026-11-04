//! Deterministic chunk delivery and measured event-emitter fixtures.
use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream,
    LlmStreamEvent,
};
use everruns_contracts::runtime::conversation::Communication;
use everruns_contracts::typed_id::TurnId;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::engine::{ReasonInput, ReasonResult};
use everruns_core::{
    AgentDefinition, Controls, Event, EventRequest, ExecutionContext, RuntimeMessage,
    event_emitter::EventEmitter,
};
use everruns_test_support::{InMemoryEventEmitter, reason_atom_with_stores};
use futures::StreamExt;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct ChunkDriver {
    chunks: usize,
    bytes: usize,
    delay: Duration,
}

#[async_trait]
impl ChatDriver for ChunkDriver {
    async fn chat_completion_stream(
        &self,
        _: &everruns_contracts::ProviderEndpoint,
        _: Vec<everruns_contracts::Message>,
        _: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        let mut events = Vec::new();
        for _ in 0..self.chunks {
            events.push(LlmStreamEvent::ReasoningDelta {
                delta: "r".repeat(self.bytes),
                summary: false,
            });
            events.push(LlmStreamEvent::TextDelta("x".repeat(self.bytes)));
        }
        let mut metadata = LlmCompletionMetadata::default();
        metadata.prompt_tokens = Some(100);
        metadata.completion_tokens = Some(200);
        metadata.finish_reason = Some("stop".into());
        events.push(LlmStreamEvent::Done(Box::new(metadata)));
        let delay = self.delay;
        Ok(Box::pin(futures::stream::iter(events).then(
            move |event| async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                Ok(event)
            },
        )))
    }
}

#[derive(Clone, Default)]
pub(super) struct MeasuredEmitter {
    pub(super) inner: InMemoryEventEmitter,
    pub(super) bytes: Arc<AtomicUsize>,
    pub(super) wait_ns: Arc<std::sync::atomic::AtomicU64>,
    pub(super) delay: Duration,
}
#[async_trait]
impl EventEmitter for MeasuredEmitter {
    async fn emit(&self, request: EventRequest) -> everruns_contracts::error::Result<Event> {
        let started = Instant::now();
        self.bytes.fetch_add(
            serde_json::to_vec(&request).unwrap().len(),
            Ordering::Relaxed,
        );
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.wait_ns
            .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.inner.emit(request).await
    }
}

pub(super) async fn run(
    stores: super::setup::TestEnvironment,
    communication: Communication,
    hint: Option<bool>,
    chunks: usize,
    delay: Duration,
    emitter: MeasuredEmitter,
) -> (ReasonResult, Vec<Event>, Duration) {
    let (h, a, s, m, p, harness_id, agent_id, session_id) = stores;
    let mut agent = AgentDefinition::new(agent_id.into(), "output-policy-test", "Write notes.");
    agent.communication = communication;
    a.add_agent(agent).await;
    let mut message = RuntimeMessage::user("Work.");
    if let Some(live) = hint {
        message.controls = Some(Controls {
            hints: Some(std::collections::HashMap::from([(
                "output_streaming".into(),
                json!(live),
            )])),
            ..Default::default()
        });
    }
    let input_message_id = message.id;
    m.seed(session_id.into(), vec![message]).await;
    let mut registry = DriverRegistry::new();
    registry.register(DriverId::LlmSim, move |_| {
        Box::new(ChunkDriver {
            chunks,
            bytes: 4096,
            delay,
        })
    });
    let atom = reason_atom_with_stores(
        h,
        a,
        s,
        m,
        p,
        CapabilityRegistry::new(),
        registry,
        emitter.clone(),
    );
    let started = Instant::now();
    let result = atom
        .execute(ReasonInput {
            context: ExecutionContext::new(session_id.into(), TurnId::new(), input_message_id),
            harness_id,
            agent_id: Some(agent_id.into()),
            org_id: 0,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        })
        .await
        .unwrap();
    let elapsed = started.elapsed();
    (result, emitter.inner.events().await, elapsed)
}
