//! The actor runtime against an in-memory actor index and the in-process
//! runtime as its host, with a scripted model.
//!
//! A worker crash is a task aborted at a chosen point (a tool that never
//! returns, or a host flush that never returns), followed by the lease
//! running out. Another worker on the same index then resumes from the log.

mod resolutions;
mod store;
mod turns;

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::tool_types::{
    ClientSideTool, SideEffectClass, ToolCall, ToolDefinition, ToolHints,
};
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{ActorHosts, ActorRuntime, ActorRuntimeConfig, HostTurn, TurnSignals};
use crate::core::capabilities::{Capability, CapabilityStatus};
use crate::core::events::{Event, EventContext, EventRequest, ToolCompletedData};
use crate::core::tools::{Tool, ToolExecutionResult};
use crate::host::{
    AcceptedTurnInput, AgentBuilder, HarnessBuilder, InProcessRuntime, OpenTurn,
    RuntimeHostAdapter, SessionBuilder, TurnDrive, TurnLogFold,
};
use store::MemoryActors;

const ORG_ID: i64 = 1;
const TEST_CAPABILITY: &str = "actor_test";
const WAIT: Duration = Duration::from_secs(10);

/// What the test tools saw, and a gate that holds their first call.
#[derive(Default)]
struct ToolGate {
    calls: Mutex<Vec<String>>,
    /// Hold the first call of any gated tool until `release`.
    hold_first: AtomicBool,
    held: Notify,
    release: Notify,
}

struct GatedTool {
    name: &'static str,
    class: SideEffectClass,
    gate: Arc<ToolGate>,
}

#[async_trait]
impl Tool for GatedTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "a tool the test can hold mid-call"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default().with_side_effect_class(self.class.clone())
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        self.gate.calls.lock().unwrap().push(self.name.to_string());
        if self.gate.hold_first.swap(false, Ordering::SeqCst) {
            self.gate.held.notify_one();
            self.gate.release.notified().await;
        }
        ToolExecutionResult::success(json!({ "tool": self.name }))
    }
}

struct GatedTools(Arc<ToolGate>);

impl Capability for GatedTools {
    fn id(&self) -> &str {
        TEST_CAPABILITY
    }
    fn name(&self) -> &str {
        "Actor test tools"
    }
    fn description(&self) -> &str {
        "Tools the actor runtime tests hold mid-call."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn features(&self) -> Vec<&'static str> {
        vec![TEST_CAPABILITY]
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        let tool = |name, class| -> Box<dyn Tool> {
            Box::new(GatedTool {
                name,
                class,
                gate: self.0.clone(),
            })
        };
        vec![
            tool("send_email", SideEffectClass::AtMostOnce),
            tool("read_inbox", SideEffectClass::Idempotent),
        ]
    }
}

type Hook = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What the hosts saw, and a flush that can be made to never return.
#[derive(Default)]
struct Probe {
    flushes: AtomicUsize,
    /// The flush (1-based) that never returns; 0 = none.
    hang_at_flush: AtomicUsize,
    hung: Notify,
    running: Mutex<HashSet<SessionId>>,
    overlapped: AtomicBool,
    on_turn_ended: Mutex<Option<Hook>>,
}

#[derive(Clone)]
struct TestHosts {
    runtime: InProcessRuntime,
    probe: Arc<Probe>,
}

#[async_trait]
impl ActorHosts for TestHosts {
    type Host = InProcessRuntime;

    fn host(&self, turn: HostTurn, _signals: &TurnSignals) -> InProcessRuntime {
        if turn.input_message_id.is_some()
            && !self.probe.running.lock().unwrap().insert(turn.session_id)
        {
            self.probe.overlapped.store(true, Ordering::SeqCst);
        }
        self.runtime.clone()
    }

    async fn flush(&self, _host: &InProcessRuntime) {
        let flush = self.probe.flushes.fetch_add(1, Ordering::SeqCst) + 1;
        if flush == self.probe.hang_at_flush.load(Ordering::SeqCst) {
            self.probe.hung.notify_one();
            std::future::pending::<()>().await;
        }
    }

    async fn turn_ended(&self, drive: &TurnDrive) {
        self.probe.running.lock().unwrap().remove(&drive.session_id);
        let hook = self.probe.on_turn_ended.lock().unwrap().take();
        if let Some(hook) = hook {
            hook.await;
        }
    }
}

/// One session on an in-process runtime, its actor index and test tools.
struct Fixture {
    runtime: InProcessRuntime,
    session_id: SessionId,
    store: Arc<MemoryActors>,
    gate: Arc<ToolGate>,
    poison_attempts: u32,
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({}),
    }
}

/// A model that makes `calls` (one batch per reason) and then answers `answer`.
fn model(answer: &str, calls: Vec<Vec<ToolCall>>) -> LlmSimConfig {
    let mut sequence = calls;
    sequence.push(Vec::new());
    LlmSimConfig::fixed(answer).with_tool_call_sequence(sequence)
}

impl Fixture {
    async fn new(model: LlmSimConfig) -> Self {
        let harness_id = HarnessId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let gate = Arc::new(ToolGate::default());
        let client_tool = |name: &str| {
            ToolDefinition::ClientSide(ClientSideTool::new(
                name,
                "Answered by a person in the client.",
                json!({ "type": "object" }),
            ))
        };
        let mut session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .tool(client_tool("confirm"))
            .tool(client_tool(
                everruns_contracts::tool_approval_types::APPROVE_TOOL_CALL_TOOL,
            ))
            .build();
        // A client is connected to answer a pause.
        session.hints = Some([("setup_connection".to_string(), json!(true))].into());
        let runtime = InProcessRuntime::builder()
            .capability(GatedTools(gate.clone()))
            .harness(
                HarnessBuilder::new("actors", "You are a test assistant.")
                    .id(harness_id)
                    .capability(TEST_CAPABILITY)
                    .build(),
            )
            .agent(
                AgentBuilder::new("actor-agent", "Use the tools when asked.")
                    .id(agent_id)
                    .max_iterations(8)
                    .build(),
            )
            .session(session)
            .llm_sim_as_default(model)
            .build()
            .await
            .expect("runtime builds");
        Self {
            store: Arc::new(MemoryActors::new(runtime.clone())),
            runtime,
            session_id,
            gate,
            poison_attempts: 3,
        }
    }

    fn worker(&self, holder: &str) -> (ActorRuntime<TestHosts>, Arc<Probe>) {
        self.worker_with_probe(holder, Arc::default())
    }

    fn worker_with_probe(
        &self,
        holder: &str,
        probe: Arc<Probe>,
    ) -> (ActorRuntime<TestHosts>, Arc<Probe>) {
        let hosts = TestHosts {
            runtime: self.runtime.clone(),
            probe: probe.clone(),
        };
        let config = ActorRuntimeConfig {
            holder: holder.to_string(),
            lease_ttl: Duration::from_secs(3),
            poison_attempts: self.poison_attempts,
            // Small pages, so the journal reads page.
            journal_page: 3,
            max_leased_per_org: None,
        };
        (ActorRuntime::new(hosts, self.store.clone(), config), probe)
    }

    /// Intake: store a person's message and mark the actor runnable.
    async fn send(&self, text: &str) -> MessageId {
        let message_id = self.store_message(text).await;
        self.mark(message_id).await;
        message_id
    }

    /// Store a message without marking the actor.
    async fn store_message(&self, text: &str) -> MessageId {
        self.runtime
            .persist_accepted_input(
                self.session_id,
                AcceptedTurnInput::new(crate::core::InputMessage::user(text)),
            )
            .await
            .expect("message stored")
    }

    /// Mark the actor runnable for a stored message.
    async fn mark(&self, message_id: MessageId) {
        let sequence = self
            .events()
            .await
            .iter()
            .find(|event| {
                matches!(&event.data, crate::core::events::EventData::InputMessage(data)
                    if data.message.id == message_id)
            })
            .and_then(|event| event.sequence)
            .expect("the message is in the log");
        self.store
            .mark_runnable(self.session_id, ORG_ID, i64::from(sequence));
    }

    /// A resolution: record `result` under the open turn, then mark.
    async fn resolve(&self, call_id: &str, tool: &str, result: Result<&str, &str>) {
        let turn = self.open_turn().await.expect("a turn is open");
        let data = match result {
            Ok(text) => ToolCompletedData::success(
                call_id.to_string(),
                tool.to_string(),
                vec![crate::core::ContentPart::text(text)],
                None,
            ),
            Err(error) => ToolCompletedData::failure(
                call_id.to_string(),
                tool.to_string(),
                "denied".to_string(),
                error.to_string(),
                None,
            ),
        };
        self.runtime
            .event_emitter()
            .emit(EventRequest::new(
                self.session_id,
                EventContext::turn(turn.turn_id(), turn.input_message_id()),
                data,
            ))
            .await
            .expect("result recorded");
        self.store.mark_resumable(self.session_id, ORG_ID).await;
    }

    async fn events(&self) -> Vec<Event> {
        self.runtime.events().await.expect("events")
    }

    async fn open_turn(&self) -> Option<OpenTurn> {
        TurnLogFold::from_events(self.events().await)
    }

    async fn types(&self) -> Vec<String> {
        self.events()
            .await
            .into_iter()
            .map(|event| event.event_type)
            .collect()
    }

    async fn count(&self, event_type: &str) -> usize {
        self.types()
            .await
            .iter()
            .filter(|seen| *seen == event_type)
            .count()
    }

    /// Assistant messages that answer (no tool calls).
    async fn answers(&self) -> Vec<String> {
        self.events()
            .await
            .into_iter()
            .filter_map(|event| match event.data {
                crate::core::events::EventData::OutputMessageCompleted(data)
                    if data.message.tool_calls().is_empty() =>
                {
                    Some(data.message.text().unwrap_or_default().to_string())
                }
                _ => None,
            })
            .collect()
    }

    fn tool_calls(&self) -> Vec<String> {
        self.gate.calls.lock().unwrap().clone()
    }

    /// The session has nothing to run and nobody holds it.
    fn assert_idle(&self) {
        let row = self.store.row(self.session_id);
        assert!(row.runnable.is_none(), "still runnable: {row:?}");
        assert!(row.holder.is_none(), "still held: {row:?}");
    }
}

/// Lease one actor on `worker` and run it; `false` when none was runnable.
async fn run_once(worker: &ActorRuntime<TestHosts>) -> bool {
    let mut leased = worker.lease(1).await.expect("lease");
    let Some(actor) = leased.pop() else {
        return false;
    };
    tokio::time::timeout(WAIT, worker.run_actor(actor))
        .await
        .expect("the actor's turn ends");
    true
}

/// Run `worker` until no actor is runnable.
async fn run_until_idle(worker: &ActorRuntime<TestHosts>) -> usize {
    let mut leases = 0;
    while run_once(worker).await {
        leases += 1;
        assert!(leases < 10, "the actor keeps being runnable");
    }
    leases
}

/// Lease and run on `worker` until `point` fires, then kill the worker and
/// let its lease run out.
async fn crash_at(fixture: &Fixture, worker: &ActorRuntime<TestHosts>, point: &Notify) {
    let worker = worker.clone();
    let run = tokio::spawn(async move { run_once(&worker).await });
    tokio::time::timeout(WAIT, point.notified())
        .await
        .expect("the worker reaches the crash point");
    run.abort();
    let _ = run.await;
    fixture.store.expire(fixture.session_id);
}

/// The event context's turn id of every `event_type` event.
fn turn_ids(
    events: &[Event],
    event_type: &str,
) -> Vec<Option<everruns_contracts::typed_id::TurnId>> {
    events
        .iter()
        .filter(|event| event.event_type == event_type)
        .map(|event| event.context.turn_id)
        .collect()
}
