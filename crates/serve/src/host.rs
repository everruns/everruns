//! The host: sessions over one `everruns::Engine`, pending approvals and
//! questions, delivery.
//!
//! Decisions:
//! - serve is a thin layer over `everruns::Engine`. The durable event log is
//!   the engine's (`Session::events_after` / `events_from`); serve authors no
//!   events of its own. Approvals are the runtime's per-tool gate
//!   (`FunctionTool::needs_approval` + `AgentBuilder::approver`) and questions
//!   are the built-in `ask_user` capability (`AgentBuilder::ask_user`); the
//!   host only parks each pending one, keyed by session and tool call id
//!   (simulated tool call ids repeat across sessions), until the wire
//!   API answers it.
//! - The binary is the behavior. Agent closures and tools cannot be
//!   serialized, so after a restart the host rebuilds the agent from the same
//!   registrations and calls `Engine::attach` + `Engine::resume`; the everruns
//!   local store supplies the conversation. Each session records the build it
//!   started on. A hosted router sends a session back to that build while it
//!   still runs; `start` refuses a session from another build (409) so the
//!   router can, and `dev` resumes it anyway because dev rebuilds constantly.
//! - Pending approvals and questions live in memory. A restart or a cancel
//!   abandons them; the runtime then sees the turn cancelled.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use anyhow::anyhow;
use everruns::approval::{ApprovalDecision, ToolApprover};
use everruns::ask_user::{
    Answer, AnsweredBy, AskContext, AskUser, Outcome, Question, QuestionKind, Status,
};
use everruns::{
    BashkitShell, FunctionTool, LocalConfig, SendDisposition, SessionEvent, SessionEventKind,
    SessionId, ToolCall, ToolCallContext, ToolDefinition, ToolResponse, TurnHandle,
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

use crate::app::{AgentEntry, App, Mode};
use crate::channel::{ChannelEvent, Inbound};
use crate::config::SandboxKind;
use crate::cx::{Cx, DeliveryTarget};
use crate::gateway;
use crate::registry::{Approval, ToolRegistration};
use crate::store::{SessionRow, Store};

/// Errors the wire API maps to specific status codes.
#[derive(Debug)]
pub(crate) enum ApiError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    /// The session belongs to another build; route it there.
    Pinned {
        build_id: String,
    },
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotFound(what) => write!(f, "{what} not found"),
            ApiError::BadRequest(why) | ApiError::Conflict(why) => f.write_str(why),
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
    pub response: String,
    pub success: bool,
    pub error: Option<String>,
}

/// A turn accepted by [`Host::send`]; `wait` for its outcome, or drop it and
/// follow the event stream instead.
pub(crate) struct PendingTurn {
    /// Id of the accepted user message.
    pub message_id: String,
    future: Pin<Box<dyn Future<Output = crate::Result<TurnOutcome>> + Send>>,
}

impl PendingTurn {
    pub(crate) async fn wait(self) -> crate::Result<TurnOutcome> {
        self.future.await
    }
}

/// What a new session is created with.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewSession {
    /// The agent to run; the default agent when `None`.
    pub agent: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub hints: Option<Value>,
    pub metadata: Option<Value>,
    /// `channel:target` to deliver replies to.
    pub deliver_to: Option<String>,
}

/// Host-side happenings that have no canonical event: the dev console and
/// in-process evals follow these. Not part of the wire API.
#[derive(Clone, Debug)]
pub(crate) enum Notice {
    /// A session is live in this process. Its canonical events after
    /// `after` are new to this process.
    Live {
        session_id: String,
        agent: String,
        resumed: bool,
        after: i32,
    },
    /// A session started on another build is resuming on this one (dev).
    BuildChanged {
        session_id: String,
        from: String,
    },
    ApprovalRequested(PendingApprovalView),
    QuestionAsked {
        session_id: String,
        tool_call_id: String,
        questions: Vec<Question>,
    },
    Delivered {
        session_id: String,
        to: String,
        error: Option<String>,
    },
}

/// A pending approval, as the wire API shows it.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PendingApprovalView {
    #[serde(skip)]
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// A pending `ask_user` question set, as the wire API shows it.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PendingQuestionView {
    pub tool_call_id: String,
    pub questions: Vec<Question>,
}

struct PendingApproval {
    view: PendingApprovalView,
    tx: oneshot::Sender<ApprovalDecision>,
}

struct PendingQuestion {
    session_id: String,
    questions: Vec<Question>,
    tx: oneshot::Sender<Outcome>,
}

/// A parked request: (session it surfaces on, tool call id).
type Key = (String, String);

struct Live {
    session: everruns::Session,
    deliver_to: Option<String>,
    active: Mutex<Option<TurnHandle>>,
}

pub(crate) struct Host {
    pub app: App,
    pub mode: Mode,
    pub build_id: String,
    pub notices: broadcast::Sender<Notice>,
    store: Store,
    engine: everruns::Engine,
    /// `Some` persists sessions through everruns' local store.
    data_dir: Option<PathBuf>,
    live: Mutex<HashMap<String, Arc<Live>>>,
    approvals: Mutex<HashMap<Key, PendingApproval>>,
    questions: Mutex<HashMap<Key, PendingQuestion>>,
    /// Question sets already answered, for `409`.
    answered: Mutex<HashSet<Key>>,
    gateway: gateway::Env,
    me: Weak<Host>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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
        let (notices, _) = broadcast::channel(1024);
        Ok(Arc::new_cyclic(|me| Host {
            app,
            mode,
            build_id,
            notices,
            store,
            engine: everruns::Engine::new(),
            data_dir,
            live: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            questions: Mutex::new(HashMap::new()),
            answered: Mutex::new(HashSet::new()),
            gateway: gateway::Env::from_process(),
            me: me.clone(),
        }))
    }

    fn arc(&self) -> crate::Result<Arc<Host>> {
        self.me
            .upgrade()
            .ok_or_else(|| anyhow!("host is shutting down"))
    }

    fn notify(&self, notice: Notice) {
        let _ = self.notices.send(notice);
    }

    pub(crate) fn session_row(&self, id: &str) -> crate::Result<SessionRow> {
        self.store
            .session(id)?
            .ok_or_else(|| ApiError::NotFound(format!("session {id}")).into())
    }

    // --- Sessions -------------------------------------------------------------

    /// Create a session on `new.agent` (or the default agent).
    pub(crate) async fn create_session(&self, new: NewSession) -> crate::Result<String> {
        let entry = match new.agent.as_deref() {
            Some(name) => self
                .app
                .agent(name)
                .filter(|entry| !entry.sub)
                .ok_or_else(|| ApiError::NotFound(format!("agent {name}")))?,
            None => self.app.default_agent().ok_or_else(|| {
                ApiError::BadRequest(
                    "this app has several agents and no default; pass agent_name".into(),
                )
            })?,
        }
        .clone();
        let agent = self.build_agent(&entry, None, true)?;
        let session = self.engine.create(agent);
        let id = session.id();
        let at = now();
        self.store.insert_session(&SessionRow {
            id: id.clone(),
            agent: entry.name.to_string(),
            build_id: self.build_id.clone(),
            title: new.title,
            tags: new.tags,
            hints: new.hints,
            metadata: new.metadata,
            created_at: at.clone(),
            updated_at: at,
            deliver_to: new.deliver_to.clone(),
        })?;
        self.go_live(&id, session, new.deliver_to);
        self.notify(Notice::Live {
            session_id: id.clone(),
            agent: entry.name.to_string(),
            resumed: false,
            after: 0,
        });
        Ok(id)
    }

    fn go_live(
        &self,
        id: &str,
        session: everruns::Session,
        deliver_to: Option<String>,
    ) -> Arc<Live> {
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
            self.notify(Notice::BuildChanged {
                session_id: id.to_string(),
                from: row.build_id.clone(),
            });
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
        let agent = self.build_agent(&entry, None, true)?;
        self.engine.attach(session_id, agent).await?;
        let session = self.engine.resume(session_id).await?;
        let after = session
            .events_after(0)
            .await?
            .last()
            .and_then(SessionEvent::sequence)
            .unwrap_or(0);
        // Two requests may race to resume; the first one in wins.
        let live = {
            let mut map = lock(&self.live);
            if let Some(existing) = map.get(id) {
                return Ok(existing.clone());
            }
            let live = Arc::new(Live {
                session,
                deliver_to: row.deliver_to,
                active: Mutex::new(None),
            });
            map.insert(id.to_string(), live.clone());
            live
        };
        self.notify(Notice::Live {
            session_id: id.to_string(),
            agent: entry.name.to_string(),
            resumed: true,
            after,
        });
        Ok(live)
    }

    /// The everruns session behind `id`, resuming it if needed.
    pub(crate) async fn session(&self, id: &str) -> crate::Result<everruns::Session> {
        Ok(self.live(id).await?.session.clone())
    }

    /// Durable canonical events of `id` with a sequence after `after`.
    pub(crate) async fn events_after(
        &self,
        id: &str,
        after: i32,
    ) -> crate::Result<Vec<SessionEvent>> {
        Ok(self.session(id).await?.events_after(after).await?)
    }

    /// `idle`, `active` while a turn runs, `waitingfortoolresults` while an
    /// approval or a question waits for a person. The everruns server status
    /// vocabulary.
    pub(crate) fn status(&self, id: &str) -> &'static str {
        let waiting = lock(&self.approvals)
            .values()
            .any(|pending| pending.view.session_id == id)
            || lock(&self.questions)
                .values()
                .any(|pending| pending.session_id == id);
        if waiting {
            return "waitingfortoolresults";
        }
        let active = lock(&self.live)
            .get(id)
            .is_some_and(|live| lock(&live.active).is_some());
        if active { "active" } else { "idle" }
    }

    pub(crate) fn pending_approvals(&self, id: &str) -> Vec<PendingApprovalView> {
        let mut pending: Vec<_> = lock(&self.approvals)
            .values()
            .filter(|pending| pending.view.session_id == id)
            .map(|pending| pending.view.clone())
            .collect();
        pending.sort_by(|a, b| a.tool_call_id.cmp(&b.tool_call_id));
        pending
    }

    pub(crate) fn pending_questions(&self, id: &str) -> Vec<PendingQuestionView> {
        let mut pending: Vec<_> = lock(&self.questions)
            .iter()
            .filter(|(_, pending)| pending.session_id == id)
            .map(|((_, tool_call_id), pending)| PendingQuestionView {
                tool_call_id: tool_call_id.clone(),
                questions: pending.questions.clone(),
            })
            .collect();
        pending.sort_by(|a, b| a.tool_call_id.cmp(&b.tool_call_id));
        pending
    }

    /// Send input: starts a turn when idle, steers the active turn otherwise.
    /// The returned future resolves when that turn ends.
    pub(crate) async fn send(&self, id: &str, input: String) -> crate::Result<PendingTurn> {
        let live = self.live(id).await?;
        let sent = live.session.send(input.as_str()).await?;
        let _ = self.store.touch(id, &now());
        let handle = sent.turn();
        if matches!(sent.disposition, SendDisposition::Started) {
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
        Ok(PendingTurn {
            message_id: sent.message_id.clone(),
            future: Box::pin(async move {
                let turn = handle.wait().await?;
                Ok(TurnOutcome {
                    response: turn.response,
                    success: turn.success,
                    error: turn.error,
                })
            }),
        })
    }

    /// Channel delivery uses the turn's final response; no event is authored.
    async fn finish_turn(
        &self,
        id: &str,
        live: &Live,
        result: Result<everruns::Turn, everruns::RunError>,
    ) {
        *lock(&live.active) = None;
        let Ok(turn) = result else { return };
        let target = live.deliver_to.as_deref().and_then(DeliveryTarget::decode);
        let (Some(target), true) = (target, turn.success && !turn.response.is_empty()) else {
            return;
        };
        let outcome = match self.app.channel(&target.channel) {
            Some(entry) => entry.channel.deliver(&target.target, &turn.response).await,
            None => Err(anyhow!("no #[channel] named `{}`", target.channel)),
        };
        self.notify(Notice::Delivered {
            session_id: id.to_string(),
            to: target.encode(),
            error: outcome.err().map(|err| format!("{err:#}")),
        });
    }

    /// Cancel the active turn. `Ok(false)` when no turn was running.
    pub(crate) async fn cancel(&self, id: &str) -> crate::Result<bool> {
        let live = self.live(id).await?;
        // A cancelled turn will never collect its pending approvals or answers.
        lock(&self.approvals).retain(|_, pending| pending.view.session_id != id);
        lock(&self.questions).retain(|_, pending| pending.session_id != id);
        let active = lock(&live.active).clone();
        match active {
            Some(turn) => Ok(turn.cancel().await.is_ok()),
            None => Ok(false),
        }
    }

    // --- Approvals and questions ---------------------------------------------

    /// Resolve a pending approval. `NotFound` when there is none by that id.
    pub(crate) fn resolve_approval(
        &self,
        session_id: &str,
        tool_call_id: &str,
        approve: bool,
    ) -> crate::Result {
        let pending =
            { lock(&self.approvals).remove(&(session_id.to_string(), tool_call_id.to_string())) };
        let Some(pending) = pending else {
            return Err(ApiError::NotFound(format!("pending approval {tool_call_id}")).into());
        };
        let decision = if approve {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Reject
        };
        let _ = pending.tx.send(decision);
        Ok(())
    }

    /// Answer a pending `ask_user` question set, like the everruns server's
    /// `POST /question-answers`. Without `tool_call_id`, the one pending set
    /// of the session is answered.
    pub(crate) fn answer_questions(
        &self,
        session_id: &str,
        tool_call_id: Option<&str>,
        status: Status,
        answers: Vec<Answer>,
    ) -> crate::Result {
        let mut questions = lock(&self.questions);
        let key = match tool_call_id {
            Some(id) => match (session_id.to_string(), id.to_string()) {
                key if questions.contains_key(&key) => key,
                key if lock(&self.answered).contains(&key) => {
                    return Err(ApiError::Conflict(
                        "This question set has already been answered".into(),
                    )
                    .into());
                }
                _ => return Err(ApiError::NotFound("pending question set".into()).into()),
            },
            None => {
                let mut mine = questions
                    .keys()
                    .filter(|(session, _)| session == session_id)
                    .cloned();
                match (mine.next(), mine.next()) {
                    (Some(id), None) => id,
                    (None, _) => {
                        return Err(ApiError::NotFound("pending question set".into()).into());
                    }
                    (Some(_), Some(_)) => {
                        return Err(ApiError::BadRequest(
                            "several question sets are pending; pass tool_call_id".into(),
                        )
                        .into());
                    }
                }
            }
        };
        let answers = match status {
            Status::Answered => {
                let asked = &questions[&key].questions;
                validate_answers(asked, &answers).map_err(ApiError::BadRequest)?;
                answers
            }
            _ => Vec::new(),
        };
        let Some(pending) = questions.remove(&key) else {
            return Err(ApiError::NotFound("pending question set".into()).into());
        };
        lock(&self.answered).insert(key);
        let _ = pending.tx.send(Outcome {
            status,
            answered_by: AnsweredBy::User,
            answers,
        });
        Ok(())
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
                            .create_session(NewSession {
                                metadata: Some(json!({ "channel": channel, "thread": thread })),
                                deliver_to: Some(DeliveryTarget::new(channel, reply_to).encode()),
                                ..NewSession::default()
                            })
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

    /// Turn a discovered agent into an `everruns::Agent`. Its approvals and
    /// questions are parked under `surface` when given (a subagent's show on
    /// its parent session), else under the session that asks.
    pub(crate) fn build_agent(
        &self,
        entry: &AgentEntry,
        surface: Option<String>,
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
        let gate = Gate {
            host: self.me.clone(),
            surface,
        };
        let mut builder = everruns::Agent::builder()
            .name(entry.name)
            .model(model)
            .instructions(instructions)
            .approver(gate.clone())
            .ask_user(gate);
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
            builder = builder.tool(self.function_tool(tool, entry.name));
        }
        if !entry.sub {
            for sub in self.app.inner.agents.iter().filter(|agent| agent.sub) {
                builder = builder.tool(self.subagent_tool(sub));
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

    /// A `#[tool]` as an everruns tool: each call gets a `Cx` over its
    /// `ToolCallContext`, and its approval rule becomes the runtime's gate.
    fn function_tool(&self, tool: &'static ToolRegistration, agent: &'static str) -> FunctionTool {
        let host = self.me.clone();
        let function = FunctionTool::with_context(
            tool.name,
            tool.description,
            (tool.schema)(),
            move |call: ToolCallContext, args: Value| {
                (tool.call)(Cx::tool(host.clone(), agent, call), args)
            },
        );
        match tool.approval {
            Approval::Never => function,
            Approval::Always => function.always_needs_approval(),
            Approval::When(predicate) => function.needs_approval(predicate),
        }
    }

    /// `ask_<name>`: run a subagent to completion on a task and return its
    /// answer. The child session's tool activity is reported as progress on
    /// the parent call, and its approvals and questions surface on the parent
    /// session.
    fn subagent_tool(&self, sub: &AgentEntry) -> FunctionTool {
        let description = sub
            .spec
            .description
            .clone()
            .or_else(|| (!sub.doc.is_empty()).then(|| sub.doc.to_string()))
            .unwrap_or_else(|| format!("Delegate a task to the {} subagent.", sub.name));
        let me = self.me.clone();
        let sub = sub.clone();
        FunctionTool::with_context(
            format!("ask_{}", sub.name),
            description,
            json!({
                "type": "object",
                "properties": { "task": { "type": "string", "description": "What the subagent should do, with all context it needs." } },
                "required": ["task"],
            }),
            move |call: ToolCallContext, args: Value| {
                let (me, sub) = (me.clone(), sub.clone());
                async move {
                    let Some(host) = me.upgrade() else {
                        return Ok::<_, String>(ToolResponse::error("host is shutting down"));
                    };
                    let task = args
                        .get("task")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let parent = call.session_id().to_string();
                    let agent = match host.build_agent(&sub, Some(parent), true) {
                        Ok(agent) => agent,
                        Err(err) => return Ok(ToolResponse::error(format!("{err:#}"))),
                    };
                    // A child session on the host's own engine, persisted in
                    // the same store as its parent.
                    let child = host.engine.create(agent);
                    drop(host);
                    let mut events = child.events();
                    let run = child.send_and_wait(task.as_str());
                    tokio::pin!(run);
                    let turn = loop {
                        tokio::select! {
                            turn = &mut run => break turn,
                            event = events.recv() => {
                                if let Ok(Some(event)) = event
                                    && let Some(line) = child_progress(sub.name, &event)
                                {
                                    call.progress(line).await;
                                }
                            }
                        }
                    };
                    Ok(match turn {
                        Ok(turn) if turn.success => ToolResponse::text(turn.response),
                        Ok(turn) => ToolResponse::error(turn.error.unwrap_or(turn.response)),
                        Err(err) => ToolResponse::error(err.to_string()),
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

/// A subagent's tool activity, as a progress line on the parent call.
fn child_progress(sub: &str, event: &SessionEvent) -> Option<String> {
    match &event.kind {
        SessionEventKind::ToolStarted { tool_name, .. } => Some(format!("{sub}: {tool_name}")),
        SessionEventKind::ToolCompleted {
            tool_name, success, ..
        } => Some(format!(
            "{sub}: {tool_name} {}",
            if *success { "done" } else { "failed" }
        )),
        SessionEventKind::ToolProgress {
            tool_name, message, ..
        } => Some(format!("{sub}: {tool_name}: {message}")),
        _ => None,
    }
}

fn load_asset(asset: &crate::registry::AssetRegistration, hot: bool) -> String {
    if hot && let Ok(text) = std::fs::read_to_string(asset.disk) {
        return text;
    }
    asset.contents.to_string()
}

/// The runtime's approver and `ask_user` responder for serve agents: park the
/// request under its tool call id until the wire API answers it.
#[derive(Clone)]
struct Gate {
    host: Weak<Host>,
    /// Where requests surface; the asking session when `None`.
    surface: Option<String>,
}

/// Removes a parked request when the waiting turn goes away (cancel, drop).
struct Parked<'a, T> {
    map: &'a Mutex<HashMap<Key, T>>,
    key: Key,
}

impl<T> Drop for Parked<'_, T> {
    fn drop(&mut self) {
        lock(self.map).remove(&self.key);
    }
}

#[everruns::approval::async_trait]
impl ToolApprover for Gate {
    async fn approve(
        &self,
        session_id: SessionId,
        call: &ToolCall,
        _definition: &ToolDefinition,
    ) -> ApprovalDecision {
        let Some(host) = self.host.upgrade() else {
            return ApprovalDecision::Unavailable;
        };
        let view = PendingApprovalView {
            session_id: self
                .surface
                .clone()
                .unwrap_or_else(|| session_id.to_string()),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
        };
        let (tx, rx) = oneshot::channel();
        let key = (view.session_id.clone(), call.id.clone());
        lock(&host.approvals).insert(
            key.clone(),
            PendingApproval {
                view: view.clone(),
                tx,
            },
        );
        let _parked = Parked {
            map: &host.approvals,
            key,
        };
        host.notify(Notice::ApprovalRequested(view));
        rx.await.unwrap_or(ApprovalDecision::Cancelled)
    }
}

#[everruns::ask_user::async_trait]
impl AskUser for Gate {
    async fn ask(&self, _questions: &[Question]) -> Outcome {
        // The runtime always asks with a context; without one nobody can answer.
        Outcome {
            status: Status::Cancelled,
            answered_by: AnsweredBy::Unattended,
            answers: Vec::new(),
        }
    }

    async fn ask_in(&self, context: &AskContext, questions: &[Question]) -> Outcome {
        let cancelled = Outcome {
            status: Status::Cancelled,
            answered_by: AnsweredBy::User,
            answers: Vec::new(),
        };
        let Some(host) = self.host.upgrade() else {
            return cancelled;
        };
        let session_id = self
            .surface
            .clone()
            .unwrap_or_else(|| context.session_id().to_string());
        let tool_call_id = context.tool_call_id().to_string();
        let key = (session_id.clone(), tool_call_id.clone());
        let (tx, rx) = oneshot::channel();
        lock(&host.questions).insert(
            key.clone(),
            PendingQuestion {
                session_id: session_id.clone(),
                questions: questions.to_vec(),
                tx,
            },
        );
        let _parked = Parked {
            map: &host.questions,
            key,
        };
        host.notify(Notice::QuestionAsked {
            session_id,
            tool_call_id,
            questions: questions.to_vec(),
        });
        rx.await.unwrap_or(cancelled)
    }
}

/// Check answers against the questions asked, as the everruns server does:
/// every question answered once, only offered options, a single-select
/// question gets one selection, and every answer says something.
fn validate_answers(questions: &[Question], answers: &[Answer]) -> Result<(), String> {
    for answer in answers {
        if !questions
            .iter()
            .any(|question| question.id.as_deref() == Some(answer.id.as_str()))
        {
            return Err(format!("no question with id {:?} was asked", answer.id));
        }
    }
    for question in questions {
        let id = question.id.as_deref().unwrap_or_default();
        let mut matching = answers.iter().filter(|answer| answer.id == id);
        let answer = matching
            .next()
            .ok_or_else(|| format!("question {id:?} was not answered"))?;
        if matching.next().is_some() {
            return Err(format!("question {id:?} was answered more than once"));
        }
        if question.kind == QuestionKind::Secret {
            return Err(format!(
                "question {id:?} asks for a secret, which serve does not store"
            ));
        }
        for label in &answer.selected {
            if !question.options.iter().any(|option| &option.label == label) {
                return Err(format!(
                    "question {id:?} was not asked with option {label:?}"
                ));
            }
        }
        let other = answer
            .other_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if other.is_some() && question.kind != QuestionKind::Text && !question.allow_other {
            return Err(format!("question {id:?} does not allow free text"));
        }
        if answer.selected.is_empty() && other.is_none() {
            return Err(format!("question {id:?} has no selection and no free text"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!("question {id:?} is single-select"));
        }
    }
    Ok(())
}

/// The event envelope the wire API sends: the complete canonical envelope,
/// with reasoning replay state stripped the way the everruns server's public
/// projection does (opaque provider parts dropped; reasoning signatures and
/// encrypted payloads removed).
pub(crate) fn wire_json(event: &SessionEvent) -> Value {
    let mut envelope = event.canonical_json().clone();
    let strip = |content: &mut Value| {
        if let Some(parts) = content.as_array_mut() {
            parts.retain(|part| part["type"] != "provider_opaque");
            for part in parts.iter_mut() {
                if part["type"] == "reasoning"
                    && let Some(part) = part.as_object_mut()
                {
                    part.remove("signature");
                    part.remove("encrypted");
                }
            }
        }
    };
    if let Some(content) = envelope.pointer_mut("/data/message/content") {
        strip(content);
    }
    if let Some(messages) = envelope
        .pointer_mut("/data/messages")
        .and_then(Value::as_array_mut)
    {
        for message in messages {
            if let Some(content) = message.get_mut("content") {
                strip(content);
            }
        }
    }
    envelope
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::ask_user::AskUserOption;

    fn question(id: &str, multi: bool) -> Question {
        Question {
            kind: QuestionKind::default(),
            id: Some(id.into()),
            header: "Target".into(),
            question: "Where?".into(),
            multi_select: multi,
            allow_other: false,
            options: vec![
                AskUserOption {
                    label: "Staging".into(),
                    description: String::new(),
                    is_default: true,
                },
                AskUserOption {
                    label: "Production".into(),
                    description: String::new(),
                    is_default: false,
                },
            ],
            secret_name: None,
            purpose: None,
        }
    }

    fn answer(id: &str, selected: &[&str]) -> Answer {
        Answer {
            id: id.into(),
            selected: selected.iter().map(|s| s.to_string()).collect(),
            other_text: None,
            secret_ref: None,
        }
    }

    #[test]
    fn answers_must_match_what_was_asked() {
        let asked = [question("target", false)];
        assert!(validate_answers(&asked, &[answer("target", &["Staging"])]).is_ok());
        assert!(validate_answers(&asked, &[answer("other", &["Staging"])]).is_err());
        assert!(validate_answers(&asked, &[]).is_err());
        assert!(validate_answers(&asked, &[answer("target", &["Moon"])]).is_err());
        assert!(validate_answers(&asked, &[answer("target", &[])]).is_err());
        assert!(validate_answers(&asked, &[answer("target", &["Staging", "Production"])]).is_err());
    }
}
