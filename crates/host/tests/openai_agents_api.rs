//! Durable OpenAI Agents API backend (EVE-1123).
//!
//! A stateful fake of the Agents API drives the turn driver through every
//! restart boundary: session create, follow-up input, function result, stream
//! disconnect, and terminal recovery. Every fake stream ends after the events
//! available so far, so each run also exercises reconnect and reconciliation.
//! A crash is a dependency failing mid-run: the in-memory driver state is
//! dropped, the lease expires, and a new driver resumes from the checkpoint.
//!
//! The live conformance test talks to OpenAI only when `OPENAI_API_KEY` is set
//! and the test is run with `--ignored`.
#![cfg(feature = "openai-agents-api")]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::agents_api_store::{
    AgentsApiCheckpoint, AgentsApiLease, AgentsApiStore, InMemoryAgentsApiStore, OutboxState,
    ToolResultState,
};
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_core::{ContentPart, RuntimeAgent, ScopedMcpServer, ScopedMcpServers};
use everruns_host::openai_agents_api::durable::{
    AgentsApiFunctionExecutor, AgentsApiLedger, AgentsApiTurnDriver, AgentsApiTurnOutcome,
    AgentsApiTurnRequest,
};
use everruns_host::openai_agents_api::{
    AgentsApiClient, AgentsApiError, FunctionCallAction, build_session_config,
};
use everruns_provider::tool_types::{ClientSideTool, ToolDefinition};
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::any};

// ---------------------------------------------------------------------------
// Fake Agents API
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeTurn {
    id: String,
    n: usize,
    status: String,
    error: Value,
    items: Vec<Value>,
    created_at: u64,
}

#[derive(Default)]
struct FakeSession {
    id: String,
    metadata: Value,
    required_actions: Vec<Value>,
    turns: Vec<FakeTurn>,
    pending: Vec<Value>,
}

#[derive(Default)]
struct FakeState {
    sessions: Vec<FakeSession>,
    turn_counter: usize,
    clock: u64,
    seen_keys: HashSet<String>,
    creates: usize,
    input_posts: usize,
    tool_result_posts: usize,
    requests: usize,
    /// Drop stream events of these types (missing provider events).
    drop: Vec<&'static str>,
    /// Send every stream event twice (duplicate provider events).
    duplicate: bool,
    /// Fail every turn with a billing error, as the live API did.
    fail_turns: bool,
}

#[derive(Clone, Default)]
struct FakeAgentsApi(Arc<Mutex<FakeState>>);

fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn conflict(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(409)
        .set_body_json(json!({"error": {"type": "conflict_error", "message": message}}))
}

impl FakeState {
    fn session(&mut self, id: &str) -> Option<&mut FakeSession> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    fn session_json(session: &FakeSession) -> Value {
        let busy = session
            .turns
            .last()
            .is_some_and(|turn| turn.status == "in_progress");
        let status = if !session.required_actions.is_empty() {
            "requires_action"
        } else if busy {
            "in_progress"
        } else {
            "idle"
        };
        json!({
            "id": session.id,
            "object": "agent.session",
            "status": status,
            "metadata": session.metadata,
            "required_actions": session.required_actions,
        })
    }

    fn turn_json(session_id: &str, turn: &FakeTurn) -> Value {
        let usage = if turn.status == "completed" {
            json!({"input_tokens": 120, "input_tokens_details": {"cached_tokens": 20}, "output_tokens": 30})
        } else {
            Value::Null
        };
        json!({
            "id": turn.id, "object": "agent.session.turn", "session_id": session_id,
            "subagent_id": null, "status": turn.status, "created_at": turn.created_at,
            "error": turn.error, "usage": usage,
        })
    }

    /// The model's first step: a commentary message, then a function call.
    fn start_turn(&mut self, session_index: usize, input: &str) {
        self.turn_counter += 1;
        self.clock += 1;
        let n = self.turn_counter;
        let fail = self.fail_turns;
        let session = &mut self.sessions[session_index];
        let sid = session.id.clone();
        let tid = format!("turn_{n}");
        let user = json!({"type": "message", "id": format!("msg_user_{n}"), "turn_id": tid, "role": "user",
            "content": [{"type": "input_text", "text": input}], "status": "completed", "phase": null});
        let mut turn = FakeTurn {
            id: tid.clone(),
            n,
            status: "in_progress".into(),
            error: Value::Null,
            items: vec![user.clone()],
            created_at: self.clock,
        };
        let ev = |kind: &str, extra: Value| {
            let mut event = json!({"type": kind, "event_id": format!("evt_{}", uuid::Uuid::new_v4().simple()),
                "session_id": sid, "turn_id": tid});
            event
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            event
        };
        let mut events = vec![
            ev(
                "agent.session.turn.created",
                json!({"turn": {"id": tid, "subagent_id": null, "status": "queued"}}),
            ),
            ev("agent.session.turn.item.added", json!({"item": user})),
        ];
        if fail {
            let error = json!({"code": "usage_limit_exceeded", "message": "Your organization has reached a usage or billing limit."});
            turn.status = "failed".into();
            turn.error = error.clone();
            events.push(
                json!({"type": "error", "event_id": "evt_err", "session_id": sid, "error": error}),
            );
            events.push(ev("agent.session.turn.failed", json!({"turn": {"id": tid, "subagent_id": null, "status": "failed", "error": error}})));
            events.push(
                json!({"type": "agent.session.idle", "session": {"id": sid, "status": "idle"}}),
            );
            session.turns.push(turn);
            session.pending.extend(events);
            return;
        }
        let commentary_id = format!("msg_commentary_{n}");
        let commentary_text = "Looking the customer up.";
        let commentary = |status: &str, text: &str| {
            json!({"type": "message", "id": commentary_id, "turn_id": tid,
            "role": "assistant", "phase": "commentary", "status": status,
            "content": [{"type": "output_text", "text": text}]})
        };
        let call_id = format!("call_{n}");
        let call = |status: &str| {
            json!({"type": "function_call", "id": call_id, "call_id": call_id, "turn_id": tid,
            "name": "lookup_customer", "arguments": {"customer_id": "123"}, "status": status})
        };
        events.push(ev(
            "agent.session.turn.item.added",
            json!({"item": commentary("in_progress", "")}),
        ));
        events.push(ev(
            "agent.session.turn.output_text.delta",
            json!({"item_id": commentary_id, "delta": commentary_text}),
        ));
        events.push(ev(
            "agent.session.turn.item.done",
            json!({"item": commentary("completed", commentary_text)}),
        ));
        events.push(ev(
            "agent.session.turn.item.added",
            json!({"item": call("in_progress")}),
        ));
        let action = json!({"type": "function_call", "turn_id": tid, "call_id": call_id,
            "name": "lookup_customer", "arguments": {"customer_id": "123"}});
        session.required_actions = vec![action.clone()];
        events.push(
            json!({"type": "agent.session.requires_action", "event_id": format!("evt_ra_{n}"),
            "session": {"id": sid, "status": "requires_action", "required_actions": [action]}}),
        );
        turn.items.push(commentary("completed", commentary_text));
        turn.items.push(call("in_progress"));
        session.turns.push(turn);
        session.pending.extend(events);
    }

    /// The model's second step: an MCP call, then the final answer.
    fn finish_turn(&mut self, session_index: usize, output: &str) {
        let session = &mut self.sessions[session_index];
        let sid = session.id.clone();
        session.required_actions.clear();
        let turn = session.turns.last_mut().unwrap();
        let (n, tid) = (turn.n, turn.id.clone());
        let ev = |kind: &str, extra: Value| {
            let mut event = json!({"type": kind, "event_id": format!("evt_{}", uuid::Uuid::new_v4().simple()),
                "session_id": sid, "turn_id": tid});
            event
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            event
        };
        let call_id = format!("call_{n}");
        let output_item = json!({"type": "function_call_output", "id": format!("fco_{n}"), "turn_id": tid,
            "call_id": call_id, "status": "completed", "output": [{"type": "input_text", "text": output}]});
        let call_done = json!({"type": "function_call", "id": call_id, "call_id": call_id, "turn_id": tid,
            "name": "lookup_customer", "arguments": {"customer_id": "123"}, "status": "completed"});
        let mcp = |status: &str| {
            json!({"type": "mcp_call", "id": format!("mcp_{n}"), "turn_id": tid,
            "server_label": "docs", "name": "search_openai_docs", "arguments": {"query": "sessions"},
            "status": status, "error": null,
            "output": if status == "completed" { json!({"content": [{"type": "text", "text": "Sessions are durable."}]}) } else { Value::Null }})
        };
        let final_id = format!("msg_final_{n}");
        let final_text = "Customer 123 is Ada; sessions are durable.";
        let final_msg = |status: &str, text: &str| {
            json!({"type": "message", "id": final_id, "turn_id": tid,
            "role": "assistant", "phase": "final_answer", "status": status,
            "content": [{"type": "output_text", "text": text}]})
        };
        let events = vec![
            ev(
                "agent.session.turn.item.added",
                json!({"item": output_item}),
            ),
            ev("agent.session.turn.item.done", json!({"item": call_done})),
            ev(
                "agent.session.turn.item.added",
                json!({"item": mcp("in_progress")}),
            ),
            ev(
                "agent.session.turn.item.done",
                json!({"item": mcp("completed")}),
            ),
            ev(
                "agent.session.turn.item.added",
                json!({"item": final_msg("in_progress", "")}),
            ),
            ev(
                "agent.session.turn.output_text.delta",
                json!({"item_id": final_id, "delta": "Customer 123 is Ada; "}),
            ),
            ev(
                "agent.session.turn.output_text.delta",
                json!({"item_id": final_id, "delta": "sessions are durable."}),
            ),
            ev(
                "agent.session.turn.item.done",
                json!({"item": final_msg("completed", final_text)}),
            ),
            ev(
                "agent.session.turn.completed",
                json!({"turn": {"id": tid, "subagent_id": null, "status": "completed", "usage": null}}),
            ),
            json!({"type": "agent.session.idle", "event_id": format!("evt_idle_{n}"), "session": {"id": sid, "status": "idle"}}),
        ];
        let call_index = turn
            .items
            .iter()
            .position(|item| item["id"] == call_id)
            .unwrap();
        turn.items[call_index] = call_done;
        turn.items.push(output_item);
        turn.items.push(mcp("completed"));
        turn.items.push(final_msg("completed", final_text));
        turn.status = "completed".into();
        session.pending.extend(events);
    }

    fn drain(&mut self, session_index: usize) -> Vec<Value> {
        let events = std::mem::take(&mut self.sessions[session_index].pending);
        let mut out = Vec::new();
        for event in events {
            let kind = event["type"].as_str().unwrap_or_default();
            if self.drop.contains(&kind) {
                continue;
            }
            if self.duplicate {
                out.push(event.clone());
            }
            out.push(event);
        }
        out
    }
}

impl Respond for FakeAgentsApi {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut state = self.0.lock().unwrap();
        state.requests += 1;
        let path = request.url.path().trim_start_matches('/').to_string();
        let query: std::collections::HashMap<String, String> =
            request.url.query_pairs().into_owned().collect();
        let segments: Vec<&str> = path.split('/').collect();
        let method = request.method.as_str();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        match (method, segments.as_slice()) {
            ("POST", ["agents", "sessions"]) => {
                state.creates += 1;
                let id = format!("sess_{}", state.creates);
                state.sessions.push(FakeSession {
                    id: id.clone(),
                    metadata: body["metadata"].clone(),
                    ..FakeSession::default()
                });
                let index = state.sessions.len() - 1;
                let session = FakeState::session_json(&state.sessions[index]);
                let mut events = vec![
                    json!({"type": "agent.session.created", "event_id": "evt_created", "session": session}),
                ];
                state.start_turn(index, body["input"].as_str().unwrap_or_default());
                events.extend(state.drain(index));
                sse(&events)
            }
            ("GET", ["agents", "sessions"]) => {
                let data: Vec<Value> = state
                    .sessions
                    .iter()
                    .rev()
                    .map(FakeState::session_json)
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(json!({"object": "list", "data": data, "has_more": false}))
            }
            ("GET", ["agents", "sessions", id]) => match state.session(id) {
                Some(session) => {
                    ResponseTemplate::new(200).set_body_json(FakeState::session_json(session))
                }
                None => ResponseTemplate::new(404),
            },
            ("GET", ["agents", "sessions", id, "events"]) => {
                let Some(index) = state.sessions.iter().position(|s| s.id == *id) else {
                    return ResponseTemplate::new(404);
                };
                let events = state.drain(index);
                sse(&events)
            }
            ("GET", ["agents", "sessions", id, "turns"]) => {
                let Some(session) = state.session(id) else {
                    return ResponseTemplate::new(404);
                };
                let sid = session.id.clone();
                let data: Vec<Value> = session
                    .turns
                    .iter()
                    .map(|t| FakeState::turn_json(&sid, t))
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(json!({"object": "list", "data": data, "has_more": false}))
            }
            ("GET", ["agents", "sessions", id, "turns", turn_id]) => {
                let Some(session) = state.session(id) else {
                    return ResponseTemplate::new(404);
                };
                let sid = session.id.clone();
                match session.turns.iter().find(|t| t.id == *turn_id) {
                    Some(turn) => {
                        ResponseTemplate::new(200).set_body_json(FakeState::turn_json(&sid, turn))
                    }
                    None => ResponseTemplate::new(404),
                }
            }
            ("GET", ["agents", "sessions", id, "items"]) => {
                let Some(session) = state.session(id) else {
                    return ResponseTemplate::new(404);
                };
                let turn_id = query.get("turn_id").cloned().unwrap_or_default();
                let data: Vec<Value> = session
                    .turns
                    .iter()
                    .filter(|t| t.id == turn_id)
                    .flat_map(|t| t.items.clone())
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(json!({"object": "list", "data": data, "has_more": false}))
            }
            ("POST", ["agents", "sessions", id, "events"]) => {
                let Some(index) = state.sessions.iter().position(|s| s.id == *id) else {
                    return ResponseTemplate::new(404);
                };
                let key = request
                    .headers
                    .get("idempotency-key")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let event = body["events"][0].clone();
                match event["type"].as_str() {
                    Some("agent.session.input.message") => state.input_posts += 1,
                    Some("agent.session.input.tool_result") => state.tool_result_posts += 1,
                    _ => {}
                }
                if let Some(key) = &key
                    && state.seen_keys.contains(key)
                {
                    return ResponseTemplate::new(202);
                }
                match event["type"].as_str() {
                    Some("agent.session.input.message") => {
                        let text = event["input"][0]["content"][0]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        state.start_turn(index, &text);
                    }
                    Some("agent.session.input.tool_result") => {
                        let session = &state.sessions[index];
                        let pending = session.required_actions.iter().any(|action| {
                            action["call_id"] == event["call_id"]
                                && action["turn_id"] == event["turn_id"]
                        });
                        if !pending {
                            return conflict("managed agent turn is not active");
                        }
                        let output = event["output"]
                            .as_str()
                            .or(event["error"].as_str())
                            .unwrap_or_default()
                            .to_string();
                        state.finish_turn(index, &output);
                    }
                    Some("agent.session.input.cancel") => {
                        if let Some(turn) = state.sessions[index].turns.last_mut() {
                            turn.status = "cancelled".into();
                        }
                    }
                    _ => return ResponseTemplate::new(400),
                }
                if let Some(key) = key {
                    state.seen_keys.insert(key);
                }
                ResponseTemplate::new(202)
            }
            _ => ResponseTemplate::new(404),
        }
    }
}

impl FakeAgentsApi {
    async fn start() -> (Self, MockServer) {
        let fake = Self::default();
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(fake.clone())
            .mount(&server)
            .await;
        (fake, server)
    }

    fn with<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Local doubles: event log, tool pipeline, and a store that can "crash"
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestLedger {
    events: Mutex<Vec<EventRequest>>,
}

impl TestLedger {
    fn data(&self) -> Vec<(String, Value)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event.event_type.clone(),
                    serde_json::to_value(&event.data).unwrap(),
                )
            })
            .collect()
    }

    fn of_type(&self, kind: &str) -> Vec<Value> {
        self.data()
            .into_iter()
            .filter(|(event_type, _)| event_type == kind)
            .map(|(_, data)| data)
            .collect()
    }

    /// Every canonical record appears exactly once.
    fn assert_each_record_once(&self) {
        let messages = self.of_type("output.message.completed");
        let ids: Vec<_> = messages
            .iter()
            .map(|m| m["message"]["id"].to_string())
            .collect();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "duplicate messages: {messages:#?}");
        let results = self.of_type("tool.completed");
        let calls: Vec<_> = results
            .iter()
            .map(|r| r["tool_call_id"].to_string())
            .collect();
        let unique: HashSet<_> = calls.iter().collect();
        assert_eq!(
            calls.len(),
            unique.len(),
            "duplicate tool results: {results:#?}"
        );
    }

    fn texts(&self) -> Vec<String> {
        self.of_type("output.message.completed")
            .iter()
            .filter_map(|m| {
                m["message"]["content"][0]["text"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect()
    }
}

#[async_trait]
impl AgentsApiLedger for TestLedger {
    async fn emit(&self, event: EventRequest) -> Result<(), AgentsApiError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    async fn has_message(
        &self,
        _: SessionId,
        message_id: MessageId,
    ) -> Result<bool, AgentsApiError> {
        let id = message_id.to_string();
        Ok(self
            .of_type("output.message.completed")
            .iter()
            .any(|m| m["message"]["id"] == id.as_str()))
    }

    async fn tool_result(
        &self,
        _: SessionId,
        call_id: &str,
    ) -> Result<Option<Result<String, String>>, AgentsApiError> {
        Ok(self
            .of_type("tool.completed")
            .into_iter()
            .find(|r| r["tool_call_id"] == call_id)
            .map(|r| match r["error"].as_str() {
                Some(error) => Err(error.to_string()),
                None => Ok(r["result"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()),
            }))
    }
}

/// Emulates the Act pipeline: records `tool.completed`, then returns.
struct TestExecutor {
    ledger: Arc<TestLedger>,
    calls: AtomicUsize,
    /// Crash after the tool ran and its result was recorded.
    crash_after_record: AtomicBool,
}

impl TestExecutor {
    fn new(ledger: Arc<TestLedger>) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            calls: AtomicUsize::new(0),
            crash_after_record: AtomicBool::new(false),
        })
    }
}

#[async_trait]
impl AgentsApiFunctionExecutor for TestExecutor {
    async fn execute(
        &self,
        call: &FunctionCallAction,
    ) -> Result<Result<String, String>, AgentsApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let output = r#"{"name":"Ada"}"#.to_string();
        self.ledger
            .emit(EventRequest::new(
                SessionId::from_seed(1),
                EventContext::empty(),
                ToolCompletedData::success(
                    call.call_id.clone(),
                    call.name.clone(),
                    vec![ContentPart::text(output.clone())],
                    None,
                ),
            ))
            .await?;
        if self.crash_after_record.swap(false, Ordering::SeqCst) {
            return Err(AgentsApiError::Store("worker crashed".into()));
        }
        Ok(Ok(output))
    }
}

type CrashPoint = Box<dyn Fn(&AgentsApiCheckpoint) -> bool + Send + Sync>;

/// In-memory store that fails one save matching a predicate, as if the worker
/// died right before persisting that state.
struct CrashingStore {
    inner: InMemoryAgentsApiStore,
    crash: Mutex<Option<CrashPoint>>,
}

impl CrashingStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryAgentsApiStore::new(),
            crash: Mutex::new(None),
        })
    }

    fn crash_when(&self, point: impl Fn(&AgentsApiCheckpoint) -> bool + Send + Sync + 'static) {
        *self.crash.lock().unwrap() = Some(Box::new(point));
    }
}

#[async_trait]
impl AgentsApiStore for CrashingStore {
    async fn acquire(
        &self,
        lease: AgentsApiLease,
    ) -> everruns_provider::error::Result<AgentsApiCheckpoint> {
        self.inner.acquire(lease).await
    }
    async fn renew(&self, lease: AgentsApiLease) -> everruns_provider::error::Result<()> {
        self.inner.renew(lease).await
    }
    async fn save(
        &self,
        lease: AgentsApiLease,
        checkpoint: &AgentsApiCheckpoint,
    ) -> everruns_provider::error::Result<()> {
        let crashed = {
            let mut crash = self.crash.lock().unwrap();
            let hit = crash.as_ref().is_some_and(|point| point(checkpoint));
            if hit {
                *crash = None;
            }
            hit
        };
        if crashed {
            return Err(everruns_provider::error::AgentLoopError::store(
                "worker crashed before save",
            ));
        }
        self.inner.save(lease, checkpoint).await
    }
    async fn release(&self, lease: AgentsApiLease) -> everruns_provider::error::Result<()> {
        self.inner.release(lease).await
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn agent() -> RuntimeAgent {
    let mut agent = RuntimeAgent::new(
        "Look the customer up, then search the docs, then answer in one sentence.",
        "gpt-6-astra",
    );
    agent
        .tools
        .push(ToolDefinition::ClientSide(ClientSideTool::new(
            "lookup_customer",
            "Look up one customer",
            json!({
                "type": "object",
                "properties": {"customer_id": {"type": "string"}},
                "required": ["customer_id"],
                "additionalProperties": false
            }),
        )));
    agent
}

fn request(turn: u128, text: &str) -> AgentsApiTurnRequest {
    let turn_id = TurnId::from_seed(100 + turn);
    let input_message_id = MessageId::from_seed(200 + turn);
    AgentsApiTurnRequest {
        org_id: 1,
        session_id: SessionId::from_seed(1),
        turn_id,
        input_message_id,
        input_text: text.to_string(),
        config: build_session_config(&agent(), &ScopedMcpServers::default(), "", None).unwrap(),
        event_context: EventContext::turn(turn_id, input_message_id),
    }
}

struct Harness {
    fake: FakeAgentsApi,
    server: MockServer,
    store: Arc<CrashingStore>,
    ledger: Arc<TestLedger>,
    executor: Arc<TestExecutor>,
}

impl Harness {
    async fn new() -> Self {
        let (fake, server) = FakeAgentsApi::start().await;
        let ledger = Arc::new(TestLedger::default());
        Self {
            fake,
            server,
            store: CrashingStore::new(),
            executor: TestExecutor::new(ledger.clone()),
            ledger,
        }
    }

    fn driver(&self) -> AgentsApiTurnDriver {
        AgentsApiTurnDriver::new(
            AgentsApiClient::new("test-key").with_base_url(self.server.uri()),
            self.store.clone(),
            self.ledger.clone(),
            self.executor.clone(),
        )
        .with_reconnect_policy(4, Duration::from_millis(1))
    }

    /// Run until the driver returns; on a crash, expire the lease and resume
    /// with a fresh driver, as a replacement worker would.
    async fn run(&self, request: &AgentsApiTurnRequest) -> (AgentsApiTurnOutcome, usize) {
        let mut crashes = 0;
        loop {
            match self.driver().run(request).await {
                Ok(outcome) => return (outcome, crashes),
                Err(error) => {
                    assert!(crashes < 3, "driver kept failing: {error}");
                    crashes += 1;
                    self.store.inner.expire(request.session_id);
                }
            }
        }
    }

    fn checkpoint(&self) -> AgentsApiCheckpoint {
        self.store.inner.snapshot(SessionId::from_seed(1)).unwrap()
    }
}

fn assert_completed(outcome: &AgentsApiTurnOutcome) {
    match outcome {
        AgentsApiTurnOutcome::Completed {
            final_message_id,
            final_text,
            usage,
            tool_calls,
        } => {
            assert!(final_message_id.is_some());
            assert_eq!(final_text, "Customer 123 is Ada; sessions are durable.");
            let usage = usage.as_ref().expect("usage read from the turn resource");
            assert_eq!(
                usage.input_tokens, 100,
                "cached tokens are a disjoint bucket"
            );
            assert_eq!(*tool_calls, 2);
        }
        other => panic!("expected a completed turn, got {other:?}"),
    }
}

/// The record one turn must leave: commentary, function call, MCP call, and
/// final answer messages, plus one result per tool, each exactly once.
fn assert_turn_record(ledger: &TestLedger, turns: usize) {
    ledger.assert_each_record_once();
    let texts = ledger.texts();
    assert_eq!(
        texts
            .iter()
            .filter(|t| *t == "Looking the customer up.")
            .count(),
        turns
    );
    assert_eq!(
        texts
            .iter()
            .filter(|t| *t == "Customer 123 is Ada; sessions are durable.")
            .count(),
        turns
    );
    assert_eq!(ledger.of_type("output.message.completed").len(), 4 * turns);
    assert_eq!(ledger.of_type("tool.completed").len(), 2 * turns);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_turn_records_each_item_once_across_stream_disconnects() {
    let h = Harness::new().await;
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 0);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.fake.with(|s| (s.creates, s.tool_result_posts)), (1, 1));
    // The session metadata lets a replacement worker adopt the session.
    let metadata = h.fake.with(|s| s.sessions[0].metadata.clone());
    assert_eq!(
        metadata["everruns_session_id"],
        SessionId::from_seed(1).to_string()
    );
    assert!(metadata["everruns_create_attempt"].is_string());
    let checkpoint = h.checkpoint();
    assert_eq!(checkpoint.provider_session_id.as_deref(), Some("sess_1"));
    let turn = checkpoint.turn.unwrap();
    assert_eq!(turn.provider_turn_id.as_deref(), Some("turn_1"));
    assert!(turn.last_event_id.is_some(), "stream cursor is persisted");
    assert!(matches!(
        turn.tool_results["call_1"].state,
        ToolResultState::Submitted { success: true, .. }
    ));
    let mcp = h.ledger.of_type("tool.completed");
    assert!(
        mcp.iter()
            .any(|r| r["tool_name"] == "mcp_docs__search_openai_docs")
    );
}

#[tokio::test]
async fn duplicate_provider_events_cannot_duplicate_messages_or_tool_effects() {
    let h = Harness::new().await;
    h.fake.with(|s| s.duplicate = true);
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
}

#[tokio::test]
async fn missing_provider_events_are_recovered_from_saved_items() {
    let h = Harness::new().await;
    // No item completions and no required-action events ever reach the stream.
    h.fake.with(|s| {
        s.drop = vec![
            "agent.session.turn.item.done",
            "agent.session.requires_action",
            "agent.session.turn.completed",
        ]
    });
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_after_uncertain_create_adopts_the_session_instead_of_creating_another() {
    let h = Harness::new().await;
    // The worker dies after the provider created the session, before saving its id.
    h.store.crash_when(|cp| cp.provider_session_id.is_some());
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(
        h.fake.with(|s| s.creates),
        1,
        "the uncertain create was adopted"
    );
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_after_input_send_does_not_start_a_second_provider_turn() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    // Second Everruns turn on the same provider session: the worker dies after
    // the input reached the provider, before recording that it did.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.turn_id == TurnId::from_seed(102)
                && turn
                    .input
                    .as_ref()
                    .is_some_and(|input| input.state == OutboxState::Delivered)
        })
    });
    let (second, crashes) = h.run(&request(2, "And again?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&second);
    let (creates, turns, input_posts) = h
        .fake
        .with(|s| (s.creates, s.sessions[0].turns.len(), s.input_posts));
    assert_eq!(creates, 1);
    assert_eq!(
        input_posts, 2,
        "the input was retried with its idempotency key"
    );
    assert_eq!(turns, 2, "the retry did not start another provider turn");
    assert_turn_record(&h.ledger, 2);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn restart_after_tool_execution_reuses_the_recorded_result() {
    let h = Harness::new().await;
    h.executor.crash_after_record.store(true, Ordering::SeqCst);
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        1,
        "the tool ran once; recovery reused its recorded result"
    );
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_after_tool_result_submission_does_not_resubmit_a_second_result() {
    let h = Harness::new().await;
    // Dies after the provider accepted the result, before recording it.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.tool_results
                .values()
                .any(|entry| matches!(entry.state, ToolResultState::Submitted { .. }))
        })
    });
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    // Recovery saw the provider's saved output item, so it never resubmitted.
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_while_recording_a_message_checks_the_log_before_emitting_again() {
    let h = Harness::new().await;
    // Dies after emitting the final answer, before marking it completed.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.items.get("msg:msg_final_1").is_some_and(|item| {
                item.state == everruns_core::agents_api_store::ItemState::Completed
            })
        })
    });
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn terminal_recovery_returns_the_saved_outcome_without_provider_calls() {
    let h = Harness::new().await;
    // Dies after the provider turn completed, before the outcome was saved.
    h.store
        .crash_when(|cp| cp.turn.as_ref().is_some_and(|turn| turn.outcome.is_some()));
    let request = request(1, "Who is customer 123?");
    let (outcome, crashes) = h.run(&request).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);

    // A replayed activity for the finished turn makes no provider call.
    let before = h.fake.with(|s| s.requests);
    let (replayed, _) = h.run(&request).await;
    assert_completed(&replayed);
    assert_eq!(h.fake.with(|s| s.requests), before);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn failed_provider_turn_reports_the_cause() {
    let h = Harness::new().await;
    h.fake.with(|s| s.fail_turns = true);
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    match outcome {
        AgentsApiTurnOutcome::Failed { code, message } => {
            assert_eq!(code.as_deref(), Some("usage_limit_exceeded"));
            assert!(message.contains("billing limit"));
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_competing_worker_cannot_drive_a_leased_session() {
    let h = Harness::new().await;
    let lease = AgentsApiLease {
        org_id: 1,
        session_id: SessionId::from_seed(1),
        owner: uuid::Uuid::new_v4(),
    };
    h.store.acquire(lease).await.unwrap();
    let error = h.driver().run(&request(1, "hi")).await.unwrap_err();
    assert!(matches!(error, AgentsApiError::Store(_)), "{error}");
    assert_eq!(
        h.fake.with(|s| s.requests),
        0,
        "no provider call without the lease"
    );
}

#[tokio::test]
async fn http_errors_surface_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(403).set_body_string(r#"{"error":{"code":"beta_access"}}"#),
        )
        .mount(&server)
        .await;
    let ledger = Arc::new(TestLedger::default());
    let driver = AgentsApiTurnDriver::new(
        AgentsApiClient::new("test-key").with_base_url(server.uri()),
        Arc::new(InMemoryAgentsApiStore::new()),
        ledger.clone(),
        TestExecutor::new(ledger),
    );
    let error = driver.run(&request(1, "hi")).await.unwrap_err();
    assert!(
        matches!(error, AgentsApiError::Api { status: 403, ref body } if body.contains("beta_access")),
        "{error}"
    );
    let auth = server.received_requests().await.unwrap()[0].clone();
    assert_eq!(
        auth.headers.get("authorization").unwrap(),
        "Bearer test-key"
    );
    assert_eq!(auth.headers.get("openai-beta").unwrap(), "agents=v1");
}

// ---------------------------------------------------------------------------
// Recorded live traffic
// ---------------------------------------------------------------------------

/// Serves a stream recorded from the live API: the create stream up to the
/// first required action, the rest on the next event stream, and saved items
/// and the turn resource derived from the recording.
#[derive(Clone)]
struct Replay {
    first: Vec<Value>,
    rest: Arc<Mutex<Option<Vec<Value>>>>,
    items: Vec<Value>,
    turn: Value,
    submitted: Arc<Mutex<Vec<Value>>>,
}

impl Replay {
    fn new(recording: &str) -> Self {
        let events: Vec<Value> = serde_json::from_str(recording).unwrap();
        let split = events
            .iter()
            .position(|e| e["type"] == "agent.session.requires_action")
            .map_or(events.len(), |i| i + 1);
        let mut items: Vec<Value> = Vec::new();
        for event in &events {
            if let Some(item) = event.get("item") {
                match items.iter_mut().find(|known| known["id"] == item["id"]) {
                    Some(known) => *known = item.clone(),
                    None => items.push(item.clone()),
                }
            }
        }
        let turn = events
            .iter()
            .rev()
            .find(|e| {
                e["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("agent.session.turn."))
                    && e["turn"]["status"].is_string()
            })
            .map(|e| e["turn"].clone())
            .unwrap();
        Self {
            first: events[..split].to_vec(),
            rest: Arc::new(Mutex::new(Some(events[split..].to_vec()))),
            items,
            turn,
            submitted: Arc::default(),
        }
    }
}

impl Respond for Replay {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let path = request.url.path().to_string();
        match (request.method.as_str(), path.as_str()) {
            ("POST", "/agents/sessions") => sse(&self.first),
            ("POST", p) if p.ends_with("/events") => {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                self.submitted.lock().unwrap().push(body);
                ResponseTemplate::new(202)
            }
            ("GET", p) if p.ends_with("/events") => {
                sse(&self.rest.lock().unwrap().take().unwrap_or_default())
            }
            ("GET", p) if p.ends_with("/items") => ResponseTemplate::new(200)
                .set_body_json(json!({"data": self.items, "has_more": false})),
            ("GET", p) if p.contains("/turns/") => {
                ResponseTemplate::new(200).set_body_json(self.turn.clone())
            }
            ("GET", p) if p.ends_with("/turns") => ResponseTemplate::new(200)
                .set_body_json(json!({"data": [self.turn], "has_more": false})),
            ("GET", _) => ResponseTemplate::new(200).set_body_json(
                json!({"id": "sess_replay", "status": "idle", "required_actions": []}),
            ),
            _ => ResponseTemplate::new(404),
        }
    }
}

async fn replay_run(
    recording: &str,
    text: &str,
) -> (
    Replay,
    AgentsApiTurnOutcome,
    Arc<TestLedger>,
    Arc<TestExecutor>,
) {
    let replay = Replay::new(recording);
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(replay.clone())
        .mount(&server)
        .await;
    let ledger = Arc::new(TestLedger::default());
    let executor = TestExecutor::new(ledger.clone());
    let driver = AgentsApiTurnDriver::new(
        AgentsApiClient::new("test-key").with_base_url(server.uri()),
        Arc::new(InMemoryAgentsApiStore::new()),
        ledger.clone(),
        executor.clone(),
    )
    .with_reconnect_policy(4, Duration::from_millis(1));
    let outcome = driver.run(&request(1, text)).await.unwrap();
    (replay, outcome, ledger, executor)
}

#[tokio::test]
async fn recorded_live_round_trip_projects_one_function_and_two_mcp_calls() {
    // Recorded from api.openai.com on 2026-09-30 (MCP outputs trimmed).
    let (replay, outcome, ledger, executor) = replay_run(
        include_str!("fixtures/agents_api_live_round_trip.json"),
        "Who is customer 123?",
    )
    .await;
    let AgentsApiTurnOutcome::Completed {
        final_text,
        usage,
        tool_calls,
        ..
    } = outcome
    else {
        panic!("expected completion, got {outcome:?}");
    };
    assert!(
        final_text.starts_with("Customer 123 is Ada Lovelace"),
        "{final_text}"
    );
    assert!(
        usage.is_none(),
        "usage was still null when the turn completed"
    );
    assert_eq!(tool_calls, 3);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let submitted = replay.submitted.lock().unwrap().clone();
    assert_eq!(submitted.len(), 1);
    let result = &submitted[0]["events"][0];
    assert_eq!(result["type"], "agent.session.input.tool_result");
    assert_eq!(
        result["call_id"],
        "exec_63e3dc63df0ba2a4da7f253c9825b12d4ea2deca00f59aab39"
    );
    assert_eq!(result["success"], true);

    ledger.assert_each_record_once();
    let messages = ledger.of_type("output.message.completed");
    let phases: Vec<_> = messages
        .iter()
        .map(|m| m["message"]["phase"].clone())
        .collect();
    assert_eq!(phases.first(), Some(&json!("commentary")));
    assert_eq!(phases.last(), Some(&json!("final_answer")));
    // Commentary, the function call, two MCP calls, and the final answer.
    assert_eq!(messages.len(), 5);
    let results = ledger.of_type("tool.completed");
    assert_eq!(results.len(), 3);
    assert!(
        results.iter().any(|r| {
            r["tool_name"] == "mcp_docs__search_openai_docs" && r["status"] == "success"
        })
    );
    assert!(
        results
            .iter()
            .any(|r| { r["tool_name"] == "mcp_docs__fetch_openai_doc" && r["status"] == "error" })
    );
}

#[tokio::test]
async fn recorded_live_failed_turn_reports_the_billing_cause() {
    // Recorded from api.openai.com on 2026-09-30, when the org had no credits.
    let (_, outcome, ledger, executor) = replay_run(
        include_str!("fixtures/agents_api_live_failed_turn.json"),
        "hi",
    )
    .await;
    assert!(
        matches!(&outcome, AgentsApiTurnOutcome::Failed { code: Some(code), .. } if code == "usage_limit_exceeded"),
        "{outcome:?}"
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(ledger.of_type("output.message.completed").is_empty());
}

// ---------------------------------------------------------------------------
// Live conformance
// ---------------------------------------------------------------------------

/// Answers the client function with fixed data and records it like Act.
struct LiveExecutor {
    ledger: Arc<TestLedger>,
    calls: AtomicUsize,
}

#[async_trait]
impl AgentsApiFunctionExecutor for LiveExecutor {
    async fn execute(
        &self,
        call: &FunctionCallAction,
    ) -> Result<Result<String, String>, AgentsApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(call.name, "lookup_customer");
        let output = r#"{"customer_id":"123","name":"Ada Lovelace"}"#.to_string();
        self.ledger
            .emit(EventRequest::new(
                SessionId::from_seed(1),
                EventContext::empty(),
                ToolCompletedData::success(
                    call.call_id.clone(),
                    call.name.clone(),
                    vec![ContentPart::text(output.clone())],
                    None,
                ),
            ))
            .await?;
        Ok(Ok(output))
    }
}

/// Credentialed conformance: one client function and one allowed MCP tool,
/// end to end through the durable driver against api.openai.com.
///
/// `doppler run -- cargo test -p everruns-host --features openai-agents-api \
///   --test openai_agents_api -- --ignored live_`
#[tokio::test]
#[ignore = "calls the paid OpenAI Agents API; needs OPENAI_API_KEY"]
async fn live_conformance_one_client_function_and_one_allowed_mcp_tool() {
    let Ok(api_key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("SKIP: OPENAI_API_KEY is not set");
        return;
    };
    let mut req = request(
        1,
        "Look up customer 123, then search the OpenAI docs for 'Agents API sessions'. Answer in one sentence.",
    );
    req.session_id = SessionId::new();
    let servers = ScopedMcpServers::from([(
        "docs".to_string(),
        ScopedMcpServer {
            url: "https://developers.openai.com/mcp".to_string(),
            ..ScopedMcpServer::default()
        },
    )]);
    req.config = build_session_config(&agent(), &servers, "", None)
        .unwrap()
        .allow_mcp_tools("docs", &["search_openai_docs"]);
    let ledger = Arc::new(TestLedger::default());
    let executor = Arc::new(LiveExecutor {
        ledger: ledger.clone(),
        calls: AtomicUsize::new(0),
    });
    let client = AgentsApiClient::new(api_key);
    let store = Arc::new(InMemoryAgentsApiStore::new());
    let driver = AgentsApiTurnDriver::new(
        client.clone(),
        store.clone(),
        ledger.clone(),
        executor.clone(),
    );
    let outcome = driver.run(&req).await;
    for (kind, data) in ledger.data() {
        eprintln!("{kind} {data}");
    }
    // Do not leave the provider session behind.
    if let Some(provider_session) = store
        .snapshot(req.session_id)
        .and_then(|checkpoint| checkpoint.provider_session_id)
    {
        client.delete_session(&provider_session).await.unwrap();
    }
    let outcome = outcome.expect("live turn reached a terminal state");
    let AgentsApiTurnOutcome::Completed { final_text, .. } = &outcome else {
        panic!("live turn did not complete: {outcome:?}");
    };
    assert!(!final_text.is_empty());
    assert_eq!(
        executor.calls.load(Ordering::SeqCst),
        1,
        "one client function call"
    );
    let mcp_results: Vec<_> = ledger
        .of_type("tool.completed")
        .into_iter()
        .filter(|result| result["tool_name"] == "mcp_docs__search_openai_docs")
        .collect();
    assert!(!mcp_results.is_empty(), "the allowed MCP tool ran");
    assert!(
        ledger
            .of_type("tool.completed")
            .iter()
            .all(|result| result["tool_name"] != "mcp_docs__fetch_openai_doc"),
        "a tool outside the allowlist ran"
    );
    ledger.assert_each_record_once();
}
