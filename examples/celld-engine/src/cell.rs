//! The cell: one session, its canonical event log, and a turn driven one
//! engine step at a time.
//!
//! Decisions:
//! - The event log is the state. History for the model is a projection of
//!   committed events, the same projection the host uses
//!   (`input.message`, `output.message.completed`, `tool.completed`).
//! - A step is one engine atom: a Reason (one model call) or an Act (one batch
//!   of tool calls). Events a step emits are written as uncommitted. The step
//!   commits its events and the next turn position in one storage
//!   transaction, so after a crash the log holds every step or none of it.
//! - Before running, a step discards uncommitted rows and bumps the turn's
//!   `attempt`. A step that finds `attempt > 0` is a re-run after a loss:
//!   only that step repeats, never one already committed.
//! - Messages wait in an inbox and enter the log only when their turn
//!   opens, so a message posted mid-turn never lands between a tool call and
//!   its result.
//! - Streaming deltas are not kept: they are live narration, and the completed
//!   message that follows them is canonical.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::driver_registry::ChatDriver;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::provider::DriverId;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use everruns_contracts::typed_id::{EventId, HarnessId, MessageId, SessionId, TurnId, WorkspaceId};
use everruns_core::engine::{ActAtom, ActInput, ReasonAtom, ReasonInput};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    Event, EventContext, EventData, EventRequest, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData, TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::{
    AssembledTurnContext, ExecutionContext, ExecutionSession, ResolvedExecutionSnapshot,
    ResolvedModelExecution, ResolvedTurnContextInput, RuntimeMessage, TurnContextRequest,
    TurnContextResolver, assemble_resolved_turn_context,
};
use serde::{Deserialize, Serialize};

use crate::agent;

/// Most model calls in one turn.
const MAX_ITERATIONS: u32 = 16;

/// Durable storage for one cell. Every method is one storage operation;
/// [`Store::commit`] is the only one that must be atomic across rows.
pub trait Store: Send + Sync {
    /// Highest sequence in the log, committed or not.
    fn last_seq(&self) -> i64;
    fn insert(&self, seq: i64, event_type: &str, json: &str, committed: bool);
    /// Committed events in sequence order, as stored JSON.
    fn committed(&self) -> Vec<String>;
    /// Delete uncommitted events; returns how many there were.
    fn discard_uncommitted(&self) -> usize;
    fn meta(&self, key: &str) -> Option<String>;
    fn set_meta(&self, key: &str, value: &str);
    fn turn(&self) -> Option<String>;
    fn set_turn(&self, turn: &str);
    /// Queue a posted message (a serialized `RuntimeMessage`).
    fn push_inbox(&self, message: &str);
    /// The oldest queued message and its inbox id.
    fn peek_inbox(&self) -> Option<(i64, String)>;
    fn inbox_len(&self) -> usize;
    /// Atomically: mark every uncommitted event committed, store `turn` (or
    /// clear it) and `meta`, and drop inbox entry `consumed`.
    fn commit(&self, turn: Option<&str>, meta: &[(&str, String)], consumed: Option<i64>);
}

/// Where the open turn is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub turn_id: TurnId,
    pub input_message_id: MessageId,
    pub iteration: u32,
    pub next: Next,
    /// Attempts at `next` that started. Reset by every commit.
    pub attempt: u32,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Next {
    Reason,
    Act {
        tool_calls: Vec<ToolCall>,
        tool_definitions: Vec<ToolDefinition>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// No open turn and no unanswered message.
    Idle,
    /// A step committed; call again.
    Stepped,
}

pub struct Cell<S: Store + 'static> {
    pub store: Arc<S>,
    pub driver: Arc<dyn ChatDriver>,
    pub model: String,
    pub tool_delay: Duration,
}

impl<S: Store + 'static> Cell<S> {
    pub fn session_id(&self) -> SessionId {
        if let Some(id) = self
            .store
            .meta("session_id")
            .and_then(|raw| raw.parse().ok())
        {
            return id;
        }
        let id = SessionId::new();
        self.store.set_meta("session_id", &id.to_string());
        id
    }

    /// Queue a user message. One row: it is durable on return.
    pub fn post_message(&self, text: &str) -> Result<RuntimeMessage> {
        let message = RuntimeMessage::user(text);
        self.store.push_inbox(&to_json(&message)?);
        Ok(message)
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        self.store
            .committed()
            .iter()
            .map(|raw| serde_json::from_str(raw).map_err(|e| AgentLoopError::store(e.to_string())))
            .collect()
    }

    pub fn open_turn(&self) -> Option<Turn> {
        self.store
            .turn()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    }

    /// Run the next step of the open turn, opening one for the oldest
    /// unanswered message first.
    pub async fn step(&self) -> Result<Progress> {
        self.store.discard_uncommitted();
        let session_id = self.session_id();
        let mut turn = match self.open_turn() {
            Some(turn) => turn,
            None => match self.start_turn(session_id)? {
                Some(()) => return Ok(Progress::Stepped),
                None => return Ok(Progress::Idle),
            },
        };
        let resumed = turn.attempt > 0;
        turn.attempt += 1;
        self.store.set_turn(&to_json(&turn)?);
        let mut meta = Vec::new();
        if resumed {
            meta.push(("resumed_steps", self.counter("resumed_steps") + 1));
        }

        let emitter = Emitter::new(self.store.clone(), session_id);
        let context = ExecutionContext::new(session_id, turn.turn_id, turn.input_message_id)
            .with_workspace_id(workspace_id());
        let next = match turn.next.clone() {
            Next::Reason => {
                meta.push(("reason_steps", self.counter("reason_steps") + 1));
                let result = self.reason(&turn, context.clone(), emitter.clone()).await?;
                if !result.success {
                    let error = result.error.unwrap_or_else(|| "reason failed".into());
                    self.finish(&emitter, &turn, Err(error)).await?;
                    None
                } else if result.has_tool_calls && turn.iteration < MAX_ITERATIONS {
                    Some(Next::Act {
                        tool_calls: result.tool_calls,
                        tool_definitions: result.tool_definitions,
                    })
                } else {
                    self.finish(&emitter, &turn, Ok(())).await?;
                    None
                }
            }
            Next::Act {
                tool_calls,
                tool_definitions,
            } => {
                meta.push(("act_steps", self.counter("act_steps") + 1));
                turn.tool_calls += tool_calls.len() as u32;
                let atom = ActAtom::new(agent::tools(self.tool_delay), emitter.clone());
                let result = atom
                    .execute(ActInput {
                        org_id: None,
                        context,
                        harness_id: harness_id(),
                        agent_id: None,
                        tool_calls,
                        tool_definitions,
                        locale: None,
                        blueprint_id: None,
                        network_access: None,
                        parallel_tool_calls: None,
                    })
                    .await?;
                if result.waiting_for_tool_results || result.blocked {
                    self.finish(&emitter, &turn, Err("turn needs client input".into()))
                        .await?;
                    None
                } else {
                    turn.iteration += 1;
                    Some(Next::Reason)
                }
            }
        };

        let meta: Vec<(&str, String)> = meta.into_iter().map(|(k, v)| (k, v.to_string())).collect();
        match next {
            Some(next) => {
                turn.next = next;
                turn.attempt = 0;
                self.store.commit(Some(&to_json(&turn)?), &meta, None);
            }
            None => self.store.commit(None, &meta, None),
        }
        Ok(Progress::Stepped)
    }

    fn counter(&self, key: &str) -> u64 {
        self.store
            .meta(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn start_turn(&self, session_id: SessionId) -> Result<Option<()>> {
        let Some((inbox_id, raw)) = self.store.peek_inbox() else {
            return Ok(None);
        };
        let message: RuntimeMessage =
            serde_json::from_str(&raw).map_err(|e| AgentLoopError::store(e.to_string()))?;
        let turn = Turn {
            turn_id: TurnId::new(),
            input_message_id: message.id,
            iteration: 1,
            next: Next::Reason,
            attempt: 0,
            tool_calls: 0,
        };
        let context = EventContext::turn(turn.turn_id, turn.input_message_id);
        let emitter = Emitter::new(self.store.clone(), session_id);
        emitter.write(EventRequest::new(
            session_id,
            context.clone(),
            InputMessageData::new(message.clone()),
        ))?;
        emitter.write(EventRequest::new(
            session_id,
            context,
            TurnStartedData {
                turn_id: turn.turn_id,
                input_message_id: turn.input_message_id,
                input_content: message.text().map(str::to_string),
                agent_id: None,
                agent_name: None,
                agent_description: None,
            },
        ))?;
        self.store
            .commit(Some(&to_json(&turn)?), &[], Some(inbox_id));
        Ok(Some(()))
    }

    async fn reason(
        &self,
        turn: &Turn,
        context: ExecutionContext,
        emitter: Emitter<S>,
    ) -> Result<everruns_core::engine::ReasonResult> {
        let harness = agent::harness();
        let session = ExecutionSession::new(context.session_id, workspace_id(), harness_id());
        let snapshot = ResolvedExecutionSnapshot::project(&harness, None, &session)?;
        let history = project(&self.events()?);
        let capabilities = agent::capabilities(self.tool_delay);
        let assembled = assemble_resolved_turn_context(
            ResolvedTurnContextInput {
                snapshot,
                messages: history.clone(),
                message_source_sequence: None,
                model: ResolvedModelExecution {
                    model: self.model.clone(),
                    provider: ProviderKey::new("chat-completions"),
                    provider_type: DriverId::OpenAICompletions,
                    driver: self.driver.clone(),
                    provider_managed_reduction_option: None,
                },
                resolved_model_id: None,
                mcp_tool_definitions: vec![],
            },
            &capabilities,
            None,
            None,
        )
        .await?;
        ReasonAtom::new(NoStores, History(history), capabilities, emitter)
            .execute_with_assembled_context(
                ReasonInput {
                    context,
                    harness_id: harness_id(),
                    agent_id: None,
                    org_id: 0,
                    mcp_tool_definitions: vec![],
                    previous_response_id: None,
                    iteration: turn.iteration,
                },
                assembled,
            )
            .await
    }

    async fn finish(
        &self,
        emitter: &Emitter<S>,
        turn: &Turn,
        outcome: std::result::Result<(), String>,
    ) -> Result<()> {
        let context = EventContext::turn(turn.turn_id, turn.input_message_id);
        let session_id = emitter.session_id;
        let request = match outcome {
            Ok(()) => EventRequest::new(
                session_id,
                context,
                TurnCompletedData {
                    turn_id: turn.turn_id,
                    iterations: turn.iteration,
                    duration_ms: None,
                    usage: None,
                    input_content: None,
                    final_message_id: None,
                    final_answer_preview: None,
                    time_to_first_token_ms: None,
                    tool_call_count: Some(turn.tool_calls),
                    llm_call_count: Some(turn.iteration),
                    status: None,
                    stop_reason: None,
                },
            ),
            Err(error) => EventRequest::new(
                session_id,
                context,
                TurnFailedData {
                    turn_id: turn.turn_id,
                    error,
                    error_code: None,
                    error_fields: None,
                    error_disclosure: None,
                },
            ),
        };
        emitter.emit(request).await.map(|_| ())
    }
}

/// The model-visible history: the host's projection of the event log.
pub fn project(events: &[Event]) -> Vec<RuntimeMessage> {
    events
        .iter()
        .filter_map(|event| match &event.data {
            EventData::InputMessage(data) => Some(data.message.clone()),
            EventData::OutputMessageCompleted(OutputMessageCompletedData { message, .. }) => {
                Some(message.clone())
            }
            EventData::ToolCompleted(data) => Some(tool_result(event, data)),
            _ => None,
        })
        .collect()
}

fn tool_result(event: &Event, data: &ToolCompletedData) -> RuntimeMessage {
    use everruns_core::ContentPart;
    let text: Vec<&str> = data
        .result
        .iter()
        .flatten()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    let result = match text.as_slice() {
        [] => None,
        [one] => Some(serde_json::from_str(one).unwrap_or_else(|_| serde_json::json!(one))),
        many => Some(serde_json::json!(many.join(""))),
    };
    let mut message = RuntimeMessage::tool_result(&data.tool_call_id, result, data.error.clone());
    message.id = MessageId::from_uuid(event.id.uuid());
    message.created_at = event.ts;
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("tool_name".into(), serde_json::json!(data.tool_name));
    message.metadata = Some(metadata);
    message
}

fn workspace_id() -> WorkspaceId {
    WorkspaceId::from_seed(1)
}

fn harness_id() -> HarnessId {
    HarnessId::from_seed(1)
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| AgentLoopError::store(e.to_string()))
}

/// Writes engine events into the log as uncommitted rows.
pub struct Emitter<S: Store> {
    store: Arc<S>,
    session_id: SessionId,
}

impl<S: Store> Clone for Emitter<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            session_id: self.session_id,
        }
    }
}

impl<S: Store> Emitter<S> {
    fn new(store: Arc<S>, session_id: SessionId) -> Self {
        Self { store, session_id }
    }

    fn write(&self, request: EventRequest) -> Result<Event> {
        let transient = request.event_type.ends_with(".delta");
        // Read at write time: inserts are synchronous, so nothing can take
        // the sequence between the read and the insert.
        let seq = if transient {
            0
        } else {
            self.store.last_seq() + 1
        };
        let event = request.into_event(EventId::new(), seq as i32);
        if !transient {
            self.store
                .insert(seq, &event.event_type, &to_json(&event)?, false);
        }
        Ok(event)
    }
}

#[async_trait]
impl<S: Store> EventEmitter for Emitter<S> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.write(request)
    }
}

#[derive(Clone)]
struct History(Vec<RuntimeMessage>);

#[async_trait]
impl MessageRetriever for History {
    async fn get(&self, _session: SessionId, id: MessageId) -> Result<Option<RuntimeMessage>> {
        Ok(self.0.iter().find(|message| message.id == id).cloned())
    }

    async fn load(&self, _session: SessionId) -> Result<Vec<RuntimeMessage>> {
        Ok(self.0.clone())
    }
}

/// The cell always hands the engine an assembled context.
struct NoStores;

#[async_trait]
impl TurnContextResolver for NoStores {
    async fn resolve_turn_context(&self, _: TurnContextRequest) -> Result<AssembledTurnContext> {
        Err(AgentLoopError::config(
            "the cell assembles every turn itself",
        ))
    }
}
