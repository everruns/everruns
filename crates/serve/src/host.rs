//! The host: sessions, the wire event log, approvals, delivery.
//!
//! Decision: the binary is the behavior. Agent closures and tools cannot be
//! serialized, so after a restart the host rebuilds the agent from the same
//! registrations and calls `Engine::attach` + `Engine::resume`; the everruns
//! local store supplies the conversation. Each session records the build it
//! started on. A hosted router sends a session back to that build while it
//! still runs; `start` refuses a session from another build (409) so the
//! router can, and `dev` resumes it anyway because dev rebuilds constantly.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::anyhow;
use everruns::{
    BashkitShell, EventStream, EventStreamError, FunctionTool, LocalConfig, SendDisposition,
    SessionEvent, SessionEventKind, SessionId, ToolResponse, TurnHandle,
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

use crate::app::{AgentEntry, App, Mode};
use crate::channel::{ChannelEvent, Inbound};
use crate::config::SandboxKind;
use crate::cx::{Cx, Decision, DeliveryTarget};
use crate::gateway;
use crate::registry::ToolRegistration;
use crate::store::{SessionRow, Store, WireEvent};

/// Errors the wire API maps to specific status codes.
#[derive(Debug)]
pub(crate) enum ApiError {
    NotFound(String),
    BadRequest(String),
    /// The session belongs to another build; route it there.
    Pinned {
        build_id: String,
    },
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotFound(what) => write!(f, "{what} not found"),
            ApiError::BadRequest(why) => f.write_str(why),
            ApiError::Pinned { build_id } => {
                write!(
                    f,
                    "session is pinned to build {build_id}; route it to that build"
                )
            }
        }
    }
}

impl std::error::Error for ApiError {}

/// The result of one turn, as the host saw it.
#[derive(Clone, Debug)]
pub(crate) struct TurnOutcome {
    pub success: bool,
    pub error: Option<String>,
}

/// A turn accepted by [`Host::send`]; `wait` for its outcome, or drop it and
/// follow the event stream instead.
pub(crate) struct PendingTurn(Pin<Box<dyn Future<Output = crate::Result<TurnOutcome>> + Send>>);

impl PendingTurn {
    pub(crate) async fn wait(self) -> crate::Result<TurnOutcome> {
        self.0.await
    }
}

struct Live {
    session: everruns::Session,
    deliver_to: Option<String>,
    active: Mutex<Option<TurnHandle>>,
}

type PendingApproval = (String, oneshot::Sender<Decision>);

pub(crate) struct Host {
    pub app: App,
    pub mode: Mode,
    pub build_id: String,
    pub events: broadcast::Sender<WireEvent>,
    store: Store,
    engine: everruns::Engine,
    /// `Some` persists sessions through everruns' local store.
    data_dir: Option<PathBuf>,
    live: Mutex<HashMap<String, Arc<Live>>>,
    approvals: Mutex<HashMap<String, PendingApproval>>,
    gateway: gateway::Env,
    me: Weak<Host>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Host {
    /// A host persisting under `data_dir` (dev, start), or in memory (eval).
    pub(crate) fn new(app: App, mode: Mode, data_dir: Option<PathBuf>) -> crate::Result<Arc<Self>> {
        let store = match &data_dir {
            Some(dir) => Store::open(&dir.join("serve.db"))?,
            None => Store::in_memory()?,
        };
        let build_id = std::env::var("SERVE_BUILD_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| crate::manifest::build_id(&app));
        let (events, _) = broadcast::channel(4096);
        Ok(Arc::new_cyclic(|me| Host {
            app,
            mode,
            build_id,
            events,
            store,
            engine: everruns::Engine::new(),
            data_dir,
            live: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            gateway: gateway::Env::from_process(),
            me: me.clone(),
        }))
    }

    fn arc(&self) -> crate::Result<Arc<Host>> {
        self.me
            .upgrade()
            .ok_or_else(|| anyhow!("host is shutting down"))
    }

    /// Append to a session's log and fan out to live subscribers.
    pub(crate) fn emit(&self, session_id: &str, kind: &str, data: Value) -> Option<WireEvent> {
        match self.store.append(session_id, kind, data) {
            Ok(event) => {
                let _ = self.events.send(event.clone());
                Some(event)
            }
            Err(err) => {
                eprintln!("serve: failed to record {kind} for {session_id}: {err:#}");
                None
            }
        }
    }

    pub(crate) fn events_after(
        &self,
        session_id: &str,
        cursor: i64,
    ) -> crate::Result<Vec<WireEvent>> {
        self.store.events_after(session_id, cursor)
    }

    pub(crate) fn session_row(&self, id: &str) -> crate::Result<SessionRow> {
        self.store
            .session(id)?
            .ok_or_else(|| ApiError::NotFound(format!("session {id}")).into())
    }

    // --- Sessions -------------------------------------------------------------

    /// Create a session on `agent` (or the default agent).
    pub(crate) async fn create_session(
        &self,
        agent: Option<&str>,
        metadata: Value,
        deliver_to: Option<String>,
    ) -> crate::Result<String> {
        let entry = match agent {
            Some(name) => self
                .app
                .agent(name)
                .filter(|entry| !entry.sub)
                .ok_or_else(|| ApiError::NotFound(format!("agent {name}")))?,
            None => self.app.default_agent().ok_or_else(|| {
                ApiError::BadRequest(
                    "this app has several agents and no default; use /v1/agents/{name}/sessions"
                        .into(),
                )
            })?,
        }
        .clone();
        let id_cell = Arc::new(OnceLock::new());
        let cx = Cx::session(&self.arc()?, entry.name, id_cell.clone());
        let agent = self.build_agent(&entry, &cx, true)?;
        let session = self.engine.create(agent);
        let id = session.id();
        let _ = id_cell.set(id.clone());
        self.store.insert_session(&SessionRow {
            id: id.clone(),
            agent: entry.name.to_string(),
            build_id: self.build_id.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            metadata: metadata.clone(),
            deliver_to: deliver_to.clone(),
        })?;
        self.go_live(&id, session, deliver_to);
        self.emit(
            &id,
            "session.created",
            json!({ "agent": entry.name, "build_id": self.build_id, "metadata": metadata }),
        );
        Ok(id)
    }

    fn go_live(
        &self,
        id: &str,
        session: everruns::Session,
        deliver_to: Option<String>,
    ) -> Arc<Live> {
        // Subscribe before anything is sent so the first turn is fully logged.
        tokio::spawn(pump(self.me.clone(), id.to_string(), session.events()));
        let live = Arc::new(Live {
            session,
            deliver_to,
            active: Mutex::new(None),
        });
        lock(&self.live).insert(id.to_string(), live.clone());
        live
    }

    /// The live session, resuming it from the local store after a restart.
    async fn live(&self, id: &str) -> crate::Result<Arc<Live>> {
        if let Some(live) = lock(&self.live).get(id).cloned() {
            return Ok(live);
        }
        let row = self.session_row(id)?;
        if row.build_id != self.build_id {
            if self.mode == Mode::Start {
                return Err(ApiError::Pinned {
                    build_id: row.build_id,
                }
                .into());
            }
            self.emit(
                id,
                "session.build_changed",
                json!({ "from": row.build_id, "to": self.build_id }),
            );
        }
        let entry = self
            .app
            .agent(&row.agent)
            .ok_or_else(|| {
                anyhow!(
                    "session {id} ran agent `{}`, which this build lacks",
                    row.agent
                )
            })?
            .clone();
        let session_id = SessionId::parse(id)
            .map_err(|err| ApiError::BadRequest(format!("bad session id {id}: {err}")))?;
        let id_cell = Arc::new(OnceLock::from(id.to_string()));
        let cx = Cx::session(&self.arc()?, entry.name, id_cell);
        let agent = self.build_agent(&entry, &cx, true)?;
        self.engine.attach(session_id, agent).await?;
        let session = self.engine.resume(session_id).await?;
        let live = self.go_live(id, session, row.deliver_to);
        self.emit(id, "session.resumed", json!({ "build_id": self.build_id }));
        Ok(live)
    }

    /// Send input: starts a turn when idle, steers the active turn otherwise.
    /// The returned future resolves when that turn ends.
    pub(crate) async fn send(&self, id: &str, input: String) -> crate::Result<PendingTurn> {
        let live = self.live(id).await?;
        let sent = live.session.send(input.as_str()).await?;
        let started = matches!(sent.disposition, SendDisposition::Started);
        self.emit(
            id,
            "message.accepted",
            json!({
                "turn_id": sent.turn_id,
                "disposition": if started { "started" } else { "steered" },
                "input": input,
            }),
        );
        let handle = sent.turn();
        if started {
            *lock(&live.active) = Some(handle.clone());
            let me = self.me.clone();
            let id = id.to_string();
            let turn = handle.clone();
            tokio::spawn(async move {
                let result = turn.wait().await;
                if let Some(host) = me.upgrade() {
                    host.finish_turn(&id, &live, result).await;
                }
            });
        }
        Ok(PendingTurn(Box::pin(async move {
            let turn = handle.wait().await?;
            Ok(TurnOutcome {
                success: turn.success,
                error: turn.error,
            })
        })))
    }

    async fn finish_turn(
        &self,
        id: &str,
        live: &Live,
        result: Result<everruns::Turn, everruns::RunError>,
    ) {
        *lock(&live.active) = None;
        let (response, success, error, tool_calls) = match &result {
            Ok(turn) => (
                turn.response.clone(),
                turn.success,
                turn.error.clone(),
                turn.tool_calls,
            ),
            Err(err) => (String::new(), false, Some(err.to_string()), 0),
        };
        // `turn.result` is the last event of a turn: give the pump a moment
        // to log the runtime's own events for it first.
        if let Ok(turn) = &result {
            for _ in 0..200 {
                if self.store.turn_logged(id, &turn.turn_id).unwrap_or(true) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        self.emit(
            id,
            "turn.result",
            json!({ "response": response, "success": success, "error": error, "tool_calls": tool_calls }),
        );
        let target = live.deliver_to.as_deref().and_then(DeliveryTarget::decode);
        if let (Some(target), true) = (target, success && !response.is_empty()) {
            let outcome = match self.app.channel(&target.channel) {
                Some(entry) => entry.channel.deliver(&target.target, &response).await,
                None => Err(anyhow!("no #[channel] named `{}`", target.channel)),
            };
            match outcome {
                Ok(()) => self.emit(id, "delivery.completed", json!({ "to": target.encode() })),
                Err(err) => self.emit(
                    id,
                    "delivery.failed",
                    json!({ "to": target.encode(), "error": format!("{err:#}") }),
                ),
            };
        }
    }

    pub(crate) async fn cancel(&self, id: &str) -> crate::Result<bool> {
        let live = self.live(id).await?;
        // A cancelled turn will never collect its pending approvals.
        lock(&self.approvals).retain(|_, (session, _)| session != id);
        let active = lock(&live.active).clone();
        match active {
            Some(turn) => Ok(turn.cancel().await.is_ok()),
            None => Ok(false),
        }
    }

    // --- Approvals ------------------------------------------------------------

    pub(crate) async fn request_approval(
        &self,
        session_id: &str,
        tool: &str,
        arguments: &Value,
    ) -> crate::Result<Decision> {
        let approval_id = format!("apr_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let (tx, rx) = oneshot::channel();
        lock(&self.approvals).insert(approval_id.clone(), (session_id.to_string(), tx));
        self.emit(
            session_id,
            "approval.requested",
            json!({ "approval_id": approval_id, "tool": tool, "arguments": arguments }),
        );
        rx.await
            .map_err(|_| anyhow!("approval {approval_id} was abandoned"))
    }

    /// Resolve a pending approval. `Ok(false)` when there is none by that id.
    pub(crate) fn resolve_approval(
        &self,
        session_id: &str,
        approval_id: &str,
        approve: bool,
        note: Option<String>,
    ) -> crate::Result<bool> {
        let pending = {
            let mut approvals = lock(&self.approvals);
            match approvals.get(approval_id) {
                Some((owner, _)) if owner == session_id => approvals.remove(approval_id),
                _ => None,
            }
        };
        let Some((_, tx)) = pending else {
            return Ok(false);
        };
        self.emit(
            session_id,
            "approval.resolved",
            json!({ "approval_id": approval_id, "decision": if approve { "approve" } else { "deny" }, "note": note }),
        );
        let decision = if approve {
            Decision::Approve
        } else {
            Decision::Deny { note }
        };
        Ok(tx.send(decision).is_ok())
    }

    // --- Channels -------------------------------------------------------------

    /// Handle a webhook. Returns the body to answer it with.
    pub(crate) async fn inbound(&self, channel: &str, inbound: Inbound) -> crate::Result<Value> {
        let entry = self
            .app
            .channel(channel)
            .ok_or_else(|| ApiError::NotFound(format!("channel {channel}")))?
            .clone();
        let event = entry
            .channel
            .receive(inbound)
            .await
            .map_err(|err| ApiError::BadRequest(format!("channel {channel}: {err:#}")))?;
        match event {
            ChannelEvent::Respond(body) => Ok(body),
            ChannelEvent::Ignore => Ok(json!({ "ok": true })),
            ChannelEvent::Message {
                thread,
                reply_to,
                text,
            } => {
                let session = match self.store.thread_session(channel, &thread)? {
                    Some(session) => session,
                    None => {
                        let session = self
                            .create_session(
                                None,
                                json!({ "channel": channel, "thread": thread }),
                                Some(DeliveryTarget::new(channel, reply_to).encode()),
                            )
                            .await?;
                        self.store.bind_thread(channel, &thread, &session)?;
                        session
                    }
                };
                // Answer the webhook now; the reply is delivered after the turn.
                self.send(&session, text).await?;
                Ok(json!({ "ok": true, "session_id": session }))
            }
        }
    }

    // --- Agent assembly -------------------------------------------------------

    /// Turn a discovered agent into an `everruns::Agent` bound to `cx`.
    pub(crate) fn build_agent(
        &self,
        entry: &AgentEntry,
        cx: &Cx,
        persistent: bool,
    ) -> crate::Result<everruns::Agent> {
        let spec = &entry.spec;
        let (model, _) = gateway::resolve(
            &spec.model,
            spec.offline.as_ref(),
            self.mode.allow_offline(),
            &self.gateway,
        )?;
        let hot = self.mode.hot_reload();
        let instructions = match &spec.instructions {
            Some(instructions) => instructions.load(hot),
            None => match self.app.asset("instructions.md") {
                Some(asset) => load_asset(asset, hot),
                None => "You are a helpful assistant.".to_string(),
            },
        };
        let mut builder = everruns::Agent::builder()
            .name(entry.name)
            .model(model)
            .instructions(instructions);
        // Skills use the built-in capability: each embedded skill is seeded
        // read-only where it looks (`.agents/skills/<name>/SKILL.md`), and the
        // agent discovers and activates them with its own tools.
        let skills = &self.app.inner.skills;
        if !skills.is_empty() {
            builder = builder.capability(everruns::Skills);
            for skill in skills {
                builder = builder.readonly_file(
                    format!(".agents/skills/{}/SKILL.md", skill.name),
                    load_asset(skill.asset, hot),
                );
            }
        }
        for tool in self.app.tools_for(entry) {
            builder = builder.tool(function_tool(tool, cx));
        }
        if !entry.sub {
            for sub in self.app.inner.agents.iter().filter(|agent| agent.sub) {
                builder = builder.tool(self.subagent_tool(sub, cx));
            }
        }
        for connection in &self.app.inner.connections {
            let Some(mcp) = &connection.value.mcp else {
                continue;
            };
            match mcp.to_everruns(connection.value.name) {
                Ok(server) => builder = builder.mcp_server(server),
                Err(err) if self.mode == Mode::Start => return Err(err),
                // Dev and evals run without the connection rather than not at all.
                Err(_) => {}
            }
        }
        builder = match self.app.inner.config.sandbox.kind {
            SandboxKind::None => builder,
            SandboxKind::Local => builder.capability(everruns::FileSystem),
            // A host supplies the microVM adapter; locally bashkit stands in.
            SandboxKind::Bashkit | SandboxKind::Microvm => builder.capability(BashkitShell::new()),
        };
        if persistent && let Some(dir) = &self.data_dir {
            builder = builder.local(LocalConfig::new(dir.join("everruns")));
        }
        if let Some(customize) = &spec.customize {
            builder = customize(builder);
        }
        builder
            .build()
            .map_err(|err| anyhow!("agent `{}`: {err}", entry.name))
    }

    /// `ask_<name>`: run a subagent to completion on a task and return its
    /// answer. The subagent's tool calls report on the parent's stream.
    fn subagent_tool(&self, sub: &AgentEntry, cx: &Cx) -> FunctionTool {
        let description = sub
            .spec
            .description
            .clone()
            .or_else(|| (!sub.doc.is_empty()).then(|| sub.doc.to_string()))
            .unwrap_or_else(|| format!("Delegate a task to the {} subagent.", sub.name));
        let me = self.me.clone();
        let sub = sub.clone();
        let cx = cx.clone();
        FunctionTool::new(
            format!("ask_{}", sub.name),
            description,
            json!({
                "type": "object",
                "properties": { "task": { "type": "string", "description": "What the subagent should do, with all context it needs." } },
                "required": ["task"],
            }),
            move |args: Value| {
                let (me, sub, cx) = (me.clone(), sub.clone(), cx.clone());
                async move {
                    let Some(host) = me.upgrade() else {
                        return Ok::<_, String>(ToolResponse::error("host is shutting down"));
                    };
                    let task = args
                        .get("task")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let parent = cx.session_id().unwrap_or_default().to_string();
                    host.emit(
                        &parent,
                        "subagent.started",
                        json!({ "agent": sub.name, "task": task }),
                    );
                    let agent = match host.build_agent(&sub, &cx, true) {
                        Ok(agent) => agent,
                        Err(err) => return Ok(ToolResponse::error(format!("{err:#}"))),
                    };
                    // A child session on the host's own engine, persisted in
                    // the same store as its parent.
                    let turn = host.engine.create(agent).send_and_wait(task.as_str()).await;
                    let (response, success) = match turn {
                        Ok(turn) => (turn.response, turn.success),
                        Err(err) => (err.to_string(), false),
                    };
                    host.emit(
                        &parent,
                        "subagent.completed",
                        json!({ "agent": sub.name, "success": success }),
                    );
                    Ok(if success {
                        ToolResponse::text(response)
                    } else {
                        ToolResponse::error(response)
                    })
                }
            },
        )
    }

    /// Run a schedule now.
    pub(crate) async fn run_schedule(&self, name: &str) -> crate::Result {
        let entry = self
            .app
            .inner
            .schedules
            .iter()
            .find(|entry| entry.registration.name == name)
            .ok_or_else(|| ApiError::NotFound(format!("schedule {name}")))?;
        (entry.registration.run)(Cx::app(&self.arc()?)).await
    }
}

fn load_asset(asset: &crate::registry::AssetRegistration, hot: bool) -> String {
    if hot && let Ok(text) = std::fs::read_to_string(asset.disk) {
        return text;
    }
    asset.contents.to_string()
}

/// Wrap a `#[tool]` as an everruns tool bound to this session's `Cx`, with
/// its approval gate in front.
fn function_tool(tool: &'static ToolRegistration, cx: &Cx) -> FunctionTool {
    let cx = cx.for_tool(tool.name);
    FunctionTool::new(
        tool.name,
        tool.description,
        (tool.schema)(),
        move |args: Value| {
            let cx = cx.clone();
            async move {
                if tool.approval.required(&args) {
                    match cx.approval(tool.name, &args).await {
                        Ok(Decision::Approve) => {}
                        Ok(Decision::Deny { note }) => {
                            let note = note.map(|note| format!(": {note}")).unwrap_or_default();
                            return Ok(ToolResponse::error(format!(
                                "A person declined this `{}` call{note}. Do not retry it unchanged.",
                                tool.name
                            )));
                        }
                        Err(err) => return Ok(ToolResponse::error(format!("{err:#}"))),
                    }
                }
                (tool.call)(cx, args).await
            }
        },
    )
}

/// Copy everruns session events into the wire log.
async fn pump(host: Weak<Host>, id: String, mut events: EventStream) {
    loop {
        let next = events.recv().await;
        let Some(host) = host.upgrade() else {
            return;
        };
        match next {
            Ok(Some(event)) => {
                if let Some((kind, data)) = wire(&event) {
                    host.emit(&id, &kind, data);
                }
            }
            Ok(None) => return,
            Err(EventStreamError::Lagged { missed }) => {
                host.emit(&id, "stream.lagged", json!({ "missed": missed }));
            }
            Err(_) => return,
        }
    }
}

/// The wire form of an everruns event: its type and reviewed data, with the
/// fields clients match on promoted. Reasoning deltas stay off the wire.
fn wire(event: &SessionEvent) -> Option<(String, Value)> {
    let mut data = event.as_json().get("data").cloned().unwrap_or(Value::Null);
    if !data.is_object() {
        data = json!({ "value": data });
    }
    if let Some(turn) = &event.turn_id {
        data["turn_id"] = json!(turn);
    }
    match &event.kind {
        SessionEventKind::ReasoningDelta { .. } => return None,
        // Tool arguments and results are the app's own data, so the wire
        // carries them (the everruns reviewed surface leaves them out).
        SessionEventKind::ToolStarted {
            tool_name,
            tool_call_id,
        } => {
            data["tool_name"] = json!(tool_name);
            data["tool_call_id"] = json!(tool_call_id);
            data["arguments"] = event.canonical_json()["data"]["tool_call"]["arguments"].clone();
        }
        SessionEventKind::ToolProgress {
            tool_name,
            tool_call_id,
            ..
        } => {
            data["tool_name"] = json!(tool_name);
            data["tool_call_id"] = json!(tool_call_id);
        }
        SessionEventKind::ToolCompleted {
            tool_name,
            tool_call_id,
            success,
        } => {
            data["tool_name"] = json!(tool_name);
            data["tool_call_id"] = json!(tool_call_id);
            data["success"] = json!(success);
            let canonical = &event.canonical_json()["data"];
            data["result"] = canonical["result"].clone();
            if !canonical["error"].is_null() {
                data["error"] = canonical["error"].clone();
            }
        }
        SessionEventKind::TextDelta { delta } => data["delta"] = json!(delta),
        _ => {}
    }
    Some((event.event_type().to_string(), data))
}
