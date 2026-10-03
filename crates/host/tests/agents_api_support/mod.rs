//! Shared doubles for the OpenAI Agents API driver tests: a stateful fake
//! of the API, an event log, an emulated tool pipeline, and a store that
//! can "crash". Each test crate uses a different subset.
#![allow(dead_code, unused_imports)]

pub use std::collections::{HashSet, VecDeque};
pub use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub use std::sync::{Arc, Mutex};
pub use std::time::Duration;

pub use async_trait::async_trait;
pub use everruns_core::agents_api_store::{
    AgentsApiCheckpoint, AgentsApiLease, AgentsApiStore, InMemoryAgentsApiStore, OutboxState,
    ParkReason, ToolResultState,
};
pub use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
pub use everruns_core::output_guardrail::{GuardrailBlock, TrippedGuardrail};
pub use everruns_core::{ContentPart, RuntimeAgent};
pub use everruns_host::openai_agents_api::build_session_config;
pub use everruns_host::openai_agents_api::durable::{
    AgentsApiFunctionExecutor, AgentsApiLedger, AgentsApiOutputPolicy, AgentsApiTurnDriver,
    AgentsApiTurnOutcome, AgentsApiTurnRequest, FunctionBatch, FunctionOutcome,
    PARKED_CALL_EXPIRED,
};
pub use everruns_host::openai_agents_api::{AgentsApiClient, AgentsApiError};
pub use everruns_provider::tool_types::{ClientSideTool, ToolCall, ToolDefinition};
pub use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
pub use serde_json::{Value, json};
pub use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::any};

// ---------------------------------------------------------------------------
// Fake Agents API
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeTurn {
    pub id: String,
    pub n: usize,
    pub status: String,
    pub error: Value,
    pub items: Vec<Value>,
    pub created_at: u64,
    /// A provider subagent turn (not root work) when set.
    pub subagent_id: Option<String>,
    /// Usage the turn reports; `None` uses the default for its status.
    pub usage: Option<Value>,
}

#[derive(Default)]
pub struct FakeSession {
    pub id: String,
    pub metadata: Value,
    pub environment: Option<Value>,
    pub required_actions: Vec<Value>,
    pub turns: Vec<FakeTurn>,
    pub pending: Vec<Value>,
}

#[derive(Default)]
pub struct FakeState {
    pub sessions: Vec<FakeSession>,
    pub turn_counter: usize,
    pub clock: u64,
    pub seen_keys: HashSet<String>,
    pub creates: usize,
    pub input_posts: usize,
    pub tool_result_posts: usize,
    pub cancel_posts: usize,
    pub requests: usize,
    /// Every session-create body, as the provider received it.
    pub create_bodies: Vec<Value>,
    /// Every submitted tool result event.
    pub tool_results: Vec<Value>,
    /// Drop stream events of these types (missing provider events).
    pub drop: Vec<&'static str>,
    /// Send every stream event twice (duplicate provider events).
    pub duplicate: bool,
    /// Fail every turn with a billing error, as the live API did.
    pub fail_turns: bool,
    /// The managed harness also delegates to a subagent, runs a hosted web
    /// search, reasons, and compacts its context before the final answer.
    pub managed_extras: bool,
    /// Raw provider boundary inputs for hostile runtime-policy tests.
    pub initial_provider_items: Vec<Value>,
    pub stream_only_provider_items: Vec<Value>,
    pub final_provider_items: Option<Vec<Value>>,
    pub initial_required_actions: Option<Vec<Value>>,
    pub hidden_subagent: bool,
    pub provider_environment: Option<Value>,
    pub pre_root_events: Vec<Value>,
    pub hide_turns: bool,
    /// Usage stays null on every turn (the provider never reports it).
    pub null_usage: bool,
    /// A cancelled or failed root turn still reports what it spent.
    pub usage_on_failure: bool,
    /// The hosted environment fails right after the turn starts; the
    /// provider never closes the root turn.
    pub environment_failure: bool,
    /// Answer every request with this status (rejected credentials, a
    /// revoked preview) and a body that echoes part of the key.
    pub reject_all: Option<u16>,
    /// Session creates fail because the model is unavailable.
    pub model_unavailable: bool,
    /// Every `DELETE /agents/sessions/{id}`, as received.
    pub deletes: Vec<String>,
}

/// Everruns provider the test turns run on.
pub const PROVIDER_KEY: &str = "01933b5a-0000-7000-8000-000000000001";

/// Ends one stream response; the events after it wait for the next stream.
pub const STREAM_BREAK: &str = "__stream_break__";

/// Subagent the managed-extras harness delegates to.
pub const SUBAGENT_ID: &str = "subagent_researcher";
/// Hidden reasoning and compacted context that must never reach the record.
pub const HIDDEN_REASONING: &str = "HIDDEN-CHAIN-OF-THOUGHT";
pub const ENCRYPTED_CONTEXT: &str = "gAAAA-ENCRYPTED-CONTEXT";

#[derive(Clone, Default)]
pub struct FakeAgentsApi(pub Arc<Mutex<FakeState>>);

pub fn sse(events: &[Value]) -> ResponseTemplate {
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

pub fn conflict(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(409)
        .set_body_json(json!({"error": {"type": "conflict_error", "message": message}}))
}

impl FakeState {
    pub fn session(&mut self, id: &str) -> Option<&mut FakeSession> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn session_json(session: &FakeSession) -> Value {
        let busy = session
            .turns
            .iter()
            .any(|turn| turn.subagent_id.is_none() && turn.status == "in_progress");
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
            "environment": session.environment,
            "required_actions": session.required_actions,
        })
    }

    pub fn turn_json(&self, session_id: &str, turn: &FakeTurn) -> Value {
        let default = json!({"input_tokens": 120, "input_tokens_details": {"cached_tokens": 20}, "output_tokens": 30});
        let usage = match &turn.usage {
            _ if self.null_usage => Value::Null,
            Some(usage) => usage.clone(),
            None if turn.status == "completed" => default,
            None if turn.status != "in_progress" && self.usage_on_failure => default,
            None => Value::Null,
        };
        json!({
            "id": turn.id, "object": "agent.session.turn", "session_id": session_id,
            "subagent_id": turn.subagent_id, "status": turn.status, "created_at": turn.created_at,
            "started_at": 1_790_000_000u64, "completed_at": 1_790_000_002u64,
            "error": turn.error, "usage": usage,
        })
    }

    /// The session's latest root turn.
    pub fn root_turn(session: &mut FakeSession) -> &mut FakeTurn {
        session
            .turns
            .iter_mut()
            .rev()
            .find(|turn| turn.subagent_id.is_none())
            .unwrap()
    }

    /// The model's first step: a commentary message, then a function call.
    pub fn start_turn(&mut self, session_index: usize, input: &str) {
        self.turn_counter += 1;
        self.clock += 1;
        let n = self.turn_counter;
        let fail = self.fail_turns;
        let environment_failure = self.environment_failure;
        let initial_items = self.initial_provider_items.clone();
        let stream_only = self.stream_only_provider_items.clone();
        let required_actions = self.initial_required_actions.clone();
        let hidden_subagent = self.hidden_subagent;
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
            ..FakeTurn::default()
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
        if environment_failure {
            events.push(
                json!({"type": "agent.session.environment.failed", "event_id": "evt_env",
                "session": {"id": sid, "status": "failed", "error": null}}),
            );
            session.turns.push(turn);
            session.pending.extend(events);
            return;
        }
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
        for item in initial_items.iter().chain(&stream_only) {
            events.push(ev("agent.session.turn.item.added", json!({"item": item})));
        }
        turn.items.extend(initial_items);
        session.required_actions = required_actions.unwrap_or_else(|| vec![action.clone()]);
        events.push(
            json!({"type": "agent.session.requires_action", "event_id": format!("evt_ra_{n}"),
            "session": {"id": sid, "status": "requires_action", "required_actions": session.required_actions}}),
        );
        turn.items.push(commentary("completed", commentary_text));
        turn.items.push(call("in_progress"));
        session.turns.push(turn);
        if hidden_subagent {
            session.turns.push(FakeTurn {
                id: "hidden_child".into(),
                subagent_id: Some("child".into()),
                created_at: self.clock,
                status: "in_progress".into(),
                ..FakeTurn::default()
            });
        }
        session.pending.extend(events);
    }

    /// The model's second step: an MCP call, then the final answer.
    pub fn finish_turn(&mut self, session_index: usize, output: &str) {
        let extras = self.managed_extras;
        let final_items = self.final_provider_items.clone();
        let session = &mut self.sessions[session_index];
        let sid = session.id.clone();
        session.required_actions.clear();
        let turn = FakeState::root_turn(session);
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
        let mut events = vec![
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
        ];
        if let Some(items) = &final_items {
            events.truncate(2);
            events.extend(
                items
                    .iter()
                    .map(|item| ev("agent.session.turn.item.done", json!({"item":item}))),
            );
        }
        let mut extra_items = Vec::new();
        let mut subagent_turn = None;
        if extras {
            let sub_tid = format!("turn_sub_{n}");
            let sub_ev = |kind: &str, status: &str| {
                json!({"type": kind, "event_id": format!("evt_{}", uuid::Uuid::new_v4().simple()),
                    "session_id": sid, "turn_id": sub_tid, "subagent_id": SUBAGENT_ID,
                    "turn": {"id": sub_tid, "subagent_id": SUBAGENT_ID, "status": status}})
            };
            let reasoning = json!({"type": "reasoning", "id": format!("rs_{n}"), "turn_id": tid,
                "status": "completed",
                "summary": [{"type": "summary_text", "text": "Checked the customer record first."}],
                "content": [{"type": "reasoning_text", "text": HIDDEN_REASONING}],
                "encrypted_content": ENCRYPTED_CONTEXT});
            let search = |status: &str| {
                json!({"type": "web_search_call", "id": format!("ws_{n}"), "turn_id": tid,
                "status": status, "action": {"type": "search", "query": "agents api sessions"}})
            };
            let compaction = json!({"type": "compaction", "id": format!("cmp_{n}"), "turn_id": tid,
                "encrypted_content": ENCRYPTED_CONTEXT});
            let sub_item = json!({"type": "message", "id": format!("msg_sub_{n}"), "turn_id": sub_tid,
                "role": "assistant", "phase": "final_answer", "status": "completed",
                "content": [{"type": "output_text", "text": "Subagent notes."}]});
            events.extend([
                sub_ev("agent.session.turn.created", "queued"),
                {
                    let mut item = sub_ev("agent.session.turn.item.done", "in_progress");
                    item["item"] = sub_item;
                    item
                },
                // The subagent ends first: this must not end the root turn.
                sub_ev("agent.session.turn.completed", "completed"),
                // A turn terminal that names no turn cannot end this one.
                json!({"type": "agent.session.turn.completed", "event_id": "evt_anonymous",
                    "session_id": sid, "turn": {"status": "completed"}}),
                // The stream drops here, while the root turn is still running.
                json!({"type": STREAM_BREAK}),
                // Unknown provider events and items are ignored.
                ev("agent.session.turn.future_progress", json!({"detail": "x"})),
                ev(
                    "agent.session.turn.item.done",
                    json!({"item": {"type": "future_item",
                    "id": format!("fut_{n}"), "turn_id": tid, "status": "completed"}}),
                ),
                ev(
                    "agent.session.turn.item.added",
                    json!({"item": search("in_progress")}),
                ),
                ev(
                    "agent.session.turn.item.done",
                    json!({"item": search("completed")}),
                ),
                ev(
                    "agent.session.turn.item.done",
                    json!({"item": reasoning.clone()}),
                ),
                ev(
                    "agent.session.turn.item.done",
                    json!({"item": compaction.clone()}),
                ),
            ]);
            extra_items = vec![search("completed"), reasoning, compaction];
            subagent_turn = Some(FakeTurn {
                id: sub_tid,
                n,
                status: "completed".into(),
                error: Value::Null,
                items: vec![],
                created_at: turn.created_at,
                subagent_id: Some(SUBAGENT_ID.into()),
                usage: Some(json!({"input_tokens": 40, "output_tokens": 8})),
            });
        }
        events.extend([
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
        ]);
        let call_index = turn
            .items
            .iter()
            .position(|item| item["id"] == call_id)
            .unwrap();
        turn.items[call_index] = call_done;
        turn.items.push(output_item);
        for item in final_items.unwrap_or_else(|| vec![mcp("completed")]) {
            if let Some(existing) = turn
                .items
                .iter_mut()
                .find(|existing| existing["id"] == item["id"])
            {
                *existing = item;
            } else {
                turn.items.push(item);
            }
        }
        turn.items.extend(extra_items);
        turn.items.push(final_msg("completed", final_text));
        // With extras the root turn ends only when its terminal event goes
        // out, so a wrongly attributed terminal is caught mid-turn.
        turn.status = if extras { "in_progress" } else { "completed" }.into();
        session.turns.extend(subagent_turn);
        session.pending.extend(events);
    }

    /// The provider gives up on the turn while a required action is open
    /// (its own timeout), as Everruns waits for a person.
    pub fn expire_turn(&mut self, session_index: usize) {
        let session = &mut self.sessions[session_index];
        session.required_actions.clear();
        let sid = session.id.clone();
        let turn = FakeState::root_turn(session);
        let error = json!({"code": "turn_timeout", "message": "The turn timed out waiting for tool results."});
        turn.status = "failed".into();
        turn.error = error.clone();
        let tid = turn.id.clone();
        session.pending.push(
            json!({"type": "agent.session.turn.failed", "event_id": "evt_expired",
            "session_id": sid, "turn_id": tid,
            "turn": {"id": tid, "subagent_id": null, "status": "failed", "error": error}}),
        );
    }

    pub fn drain(&mut self, session_index: usize) -> Vec<Value> {
        let mut events: VecDeque<Value> =
            std::mem::take(&mut self.sessions[session_index].pending).into();
        let mut out = Vec::new();
        while let Some(event) = events.pop_front() {
            let kind = event["type"].as_str().unwrap_or_default();
            if kind == STREAM_BREAK {
                self.sessions[session_index].pending = events.into();
                break;
            }
            if kind == "agent.session.turn.completed" && event["subagent_id"].is_null() {
                let turn_id = event["turn_id"].as_str().unwrap_or_default().to_string();
                if let Some(turn) = self.sessions[session_index]
                    .turns
                    .iter_mut()
                    .find(|turn| turn.id == turn_id && turn.subagent_id.is_none())
                {
                    turn.status = "completed".into();
                }
            }
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
        if let Some(status) = state.reject_all {
            return ResponseTemplate::new(status).set_body_json(json!({
                "error": {"message": "Incorrect API key provided: sk-proj-****abcd"}
            }));
        }
        match (method, segments.as_slice()) {
            ("POST", ["agents", "sessions"]) if state.model_unavailable => {
                ResponseTemplate::new(400).set_body_json(json!({
                    "error": {"code": "model_not_found", "message": "The model does not exist"}
                }))
            }
            ("DELETE", ["agents", "sessions", id]) => {
                state.deletes.push(id.to_string());
                match state.sessions.iter().position(|s| s.id == *id) {
                    Some(index) => {
                        state.sessions.remove(index);
                        ResponseTemplate::new(200).set_body_json(json!({"id": id, "deleted": true}))
                    }
                    None => ResponseTemplate::new(404),
                }
            }
            ("POST", ["agents", "sessions"]) => {
                state.creates += 1;
                state.create_bodies.push(body.clone());
                let id = format!("sess_{}", state.creates);
                let provider_environment = state.provider_environment.clone();
                state.sessions.push(FakeSession {
                    id: id.clone(),
                    metadata: body["metadata"].clone(),
                    environment: provider_environment,
                    ..FakeSession::default()
                });
                let index = state.sessions.len() - 1;
                let session = FakeState::session_json(&state.sessions[index]);
                let mut events = vec![
                    json!({"type": "agent.session.created", "event_id": "evt_created", "session": session}),
                ];
                events.extend(state.pre_root_events.clone());
                // A seeded create sends user messages; the last is the turn's.
                let input = match &body["input"] {
                    Value::Array(messages) => messages
                        .last()
                        .and_then(|message| message["content"][0]["text"].as_str())
                        .unwrap_or_default()
                        .to_string(),
                    other => other.as_str().unwrap_or_default().to_string(),
                };
                state.start_turn(index, &input);
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
                let session =
                    &state.sessions[state.sessions.iter().position(|s| s.id == sid).unwrap()];
                let data: Vec<Value> = if state.hide_turns {
                    Vec::new()
                } else {
                    session
                        .turns
                        .iter()
                        .map(|t| state.turn_json(&sid, t))
                        .collect()
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"object": "list", "data": data, "has_more": false}))
            }
            ("GET", ["agents", "sessions", id, "turns", turn_id]) => {
                let Some(session) = state.session(id) else {
                    return ResponseTemplate::new(404);
                };
                let sid = session.id.clone();
                let session =
                    &state.sessions[state.sessions.iter().position(|s| s.id == sid).unwrap()];
                match session.turns.iter().find(|t| t.id == *turn_id) {
                    Some(turn) => {
                        ResponseTemplate::new(200).set_body_json(state.turn_json(&sid, turn))
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
                    Some("agent.session.input.tool_result") => {
                        state.tool_result_posts += 1;
                        state.tool_results.push(event.clone());
                    }
                    Some("agent.session.input.cancel") => state.cancel_posts += 1,
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
                        let session = &mut state.sessions[index];
                        session.required_actions.clear();
                        if let Some(turn) = session
                            .turns
                            .iter_mut()
                            .rev()
                            .find(|turn| turn.subagent_id.is_none())
                            && turn.status == "in_progress"
                        {
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
    pub async fn start() -> (Self, MockServer) {
        let fake = Self::default();
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(fake.clone())
            .mount(&server)
            .await;
        (fake, server)
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Local doubles: event log, tool pipeline, and a store that can "crash"
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TestLedger {
    pub events: Mutex<Vec<EventRequest>>,
}

impl TestLedger {
    pub fn data(&self) -> Vec<(String, Value)> {
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

    pub fn of_type(&self, kind: &str) -> Vec<Value> {
        self.data()
            .into_iter()
            .filter(|(event_type, _)| event_type == kind)
            .map(|(_, data)| data)
            .collect()
    }

    /// Every canonical record appears exactly once.
    pub fn assert_each_record_once(&self) {
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
        let hosted = self.of_type("tool.hosted_call");
        let lifecycles: Vec<_> = hosted
            .iter()
            .map(|call| (call["call_id"].to_string(), call["status"].to_string()))
            .collect();
        let unique: HashSet<_> = lifecycles.iter().collect();
        assert_eq!(
            lifecycles.len(),
            unique.len(),
            "duplicate hosted lifecycle: {hosted:#?}"
        );
    }

    pub fn texts(&self) -> Vec<String> {
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

/// What the emulated tool pipeline does with the next batch.
pub enum Script {
    /// Run every call and record its result.
    Run,
    /// The pipeline parks every call: an approval gate records its
    /// `tool_approval_required` failure; a client-side call records nothing.
    Park(ParkReason),
    /// Policy refuses to run more tools in this turn.
    Halt(&'static str, &'static str),
}

/// Emulates the Act pipeline: records `tool.completed`, then returns.
pub struct TestExecutor {
    pub ledger: Arc<TestLedger>,
    /// Calls that actually ran.
    pub calls: AtomicUsize,
    /// Batches the pipeline received.
    pub batches: AtomicUsize,
    /// Local ids of the calls that ran.
    pub ran: Mutex<Vec<String>>,
    pub script: Mutex<VecDeque<Script>>,
    /// Crash after the tool ran and its result was recorded.
    pub crash_after_record: AtomicBool,
}

impl TestExecutor {
    pub fn new(ledger: Arc<TestLedger>) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            calls: AtomicUsize::new(0),
            batches: AtomicUsize::new(0),
            ran: Mutex::default(),
            script: Mutex::default(),
            crash_after_record: AtomicBool::new(false),
        })
    }

    pub fn then(&self, script: Script) {
        self.script.lock().unwrap().push_back(script);
    }

    pub async fn record(
        &self,
        call: &ToolCall,
        result: Result<String, String>,
    ) -> Result<(), AgentsApiError> {
        let data = match result {
            Ok(output) => ToolCompletedData::success(
                call.id.clone(),
                call.name.clone(),
                vec![ContentPart::text(output)],
                None,
            ),
            Err(error) => ToolCompletedData::failure(
                call.id.clone(),
                call.name.clone(),
                "error".to_string(),
                error,
                None,
            ),
        };
        self.ledger
            .emit(EventRequest::new(
                SessionId::from_seed(1),
                EventContext::empty(),
                data,
            ))
            .await
    }
}

#[async_trait]
impl AgentsApiFunctionExecutor for TestExecutor {
    async fn execute(&self, calls: &[ToolCall]) -> Result<FunctionBatch, AgentsApiError> {
        self.batches.fetch_add(1, Ordering::SeqCst);
        let script = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Script::Run);
        match script {
            Script::Halt(code, message) => {
                for call in calls {
                    self.record(call, Err(message.to_string())).await?;
                }
                Ok(FunctionBatch::Halt {
                    code: code.to_string(),
                    message: message.to_string(),
                })
            }
            Script::Park(reason) => {
                if matches!(reason, ParkReason::Approval { .. }) {
                    for call in calls {
                        self.record(call, Err("tool_approval_required".to_string()))
                            .await?;
                    }
                }
                Ok(FunctionBatch::Outcomes(
                    calls
                        .iter()
                        .map(|_| FunctionOutcome::Parked(reason.clone()))
                        .collect(),
                ))
            }
            Script::Run => {
                let output = r#"{"name":"Ada"}"#.to_string();
                for call in calls {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    self.ran.lock().unwrap().push(call.id.clone());
                    self.record(call, Ok(output.clone())).await?;
                }
                if self.crash_after_record.swap(false, Ordering::SeqCst) {
                    return Err(AgentsApiError::Store("worker crashed".into()));
                }
                Ok(FunctionBatch::Outcomes(
                    calls
                        .iter()
                        .map(|_| FunctionOutcome::Done(Ok(output.clone())))
                        .collect(),
                ))
            }
        }
    }
}

pub type CrashPoint = Box<dyn Fn(&AgentsApiCheckpoint) -> bool + Send + Sync>;

/// In-memory store that fails one save matching a predicate, as if the worker
/// died right before persisting that state.
pub struct CrashingStore {
    pub inner: InMemoryAgentsApiStore,
    pub crash: Mutex<Option<CrashPoint>>,
}

impl CrashingStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryAgentsApiStore::new(),
            crash: Mutex::new(None),
        })
    }

    pub fn crash_when(&self, point: impl Fn(&AgentsApiCheckpoint) -> bool + Send + Sync + 'static) {
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

pub fn agent() -> RuntimeAgent {
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

pub fn request(turn: u128, text: &str) -> AgentsApiTurnRequest {
    let turn_id = TurnId::from_seed(100 + turn);
    let input_message_id = MessageId::from_seed(200 + turn);
    AgentsApiTurnRequest {
        org_id: 1,
        session_id: SessionId::from_seed(1),
        turn_id,
        input_message_id,
        iteration: 1,
        input_text: text.to_string(),
        seed: None,
        config: build_session_config(&agent(), "", None).unwrap(),
        event_context: EventContext::turn(turn_id, input_message_id),
        provider: Some("openai".to_string()),
        provider_key: Some(PROVIDER_KEY.to_string()),
        tools: agent().tools.iter().map(Into::into).collect(),
    }
}

pub struct Harness {
    pub fake: FakeAgentsApi,
    pub server: MockServer,
    pub store: Arc<CrashingStore>,
    pub ledger: Arc<TestLedger>,
    pub executor: Arc<TestExecutor>,
    pub policy: Mutex<Option<Arc<dyn AgentsApiOutputPolicy>>>,
}

impl Harness {
    pub async fn new() -> Self {
        let (fake, server) = FakeAgentsApi::start().await;
        let ledger = Arc::new(TestLedger::default());
        Self {
            fake,
            server,
            store: CrashingStore::new(),
            executor: TestExecutor::new(ledger.clone()),
            ledger,
            policy: Mutex::new(None),
        }
    }

    pub fn driver(&self) -> AgentsApiTurnDriver {
        let driver = AgentsApiTurnDriver::new(
            AgentsApiClient::new("test-key").with_base_url(self.server.uri()),
            self.store.clone(),
            self.ledger.clone(),
            self.executor.clone(),
        )
        .with_reconnect_policy(4, Duration::from_millis(1))
        .with_usage_poll(2, Duration::from_millis(1));
        match self.policy.lock().unwrap().clone() {
            Some(policy) => driver.with_output_policy(policy),
            None => driver,
        }
    }

    /// Run until the driver returns; on a crash, expire the lease and resume
    /// with a fresh driver, as a replacement worker would.
    pub async fn run(&self, request: &AgentsApiTurnRequest) -> (AgentsApiTurnOutcome, usize) {
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

    pub fn checkpoint(&self) -> AgentsApiCheckpoint {
        self.store.inner.snapshot(SessionId::from_seed(1)).unwrap()
    }
}

pub fn assert_completed(outcome: &AgentsApiTurnOutcome) {
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

/// One turn leaves commentary, a client function call, and a final answer;
/// the function has an Act result, while MCP has a hosted lifecycle.
pub fn assert_turn_record(ledger: &TestLedger, turns: usize) {
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
    assert_eq!(ledger.of_type("output.message.completed").len(), 3 * turns);
    assert_eq!(ledger.of_type("tool.completed").len(), turns);
    let mcp: Vec<_> = ledger
        .of_type("tool.hosted_call")
        .into_iter()
        .filter(|call| call["tool_name"] == "mcp_docs__search_openai_docs")
        .collect();
    assert_eq!(mcp.len(), 2 * turns);
}
