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
//! - Pending approvals and questions are parked in memory, and the event
//!   log is what survives a restart. A turn waiting on a person waits inside
//!   its act, and a killed process leaves that turn in the log without an
//!   end. When the session next comes back, the host asks the engine for that
//!   interrupted turn and, if every unfinished call is one that waits on a
//!   person (an approval-gated call, or `ask_user`), resumes it: the calls
//!   run again, so each parks here again under its old tool call id, and an
//!   answer lets the turn finish as it would have. A turn cut off while a
//!   call executed is left alone, so an ungated tool never runs twice.
//! - A cancel abandons the pending approvals and questions; the runtime then
//!   sees the turn cancelled.

mod agent;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use anyhow::anyhow;
use everruns::approval::{ApprovalDecision, ToolApprover};
use everruns::ask_user::{Answer, AnsweredBy, AskContext, AskUser, Outcome, Question, Status};
use everruns::{
    BashkitShell, FunctionTool, LocalConfig, SendDisposition, SentMessage, SessionEvent,
    SessionEventKind, SessionId, ToolCall, ToolCallContext, ToolDefinition, ToolResponse,
    TurnHandle,
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

use crate::app::{AgentEntry, App, Mode};
use crate::config::SandboxKind;
use crate::cx::Cx;
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
    /// Caller-retained creation identity, when supplied by the wire API.
    pub id: Option<SessionId>,
    /// The agent to run; the default agent when `None`.
    pub agent: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub hints: Option<Value>,
    pub metadata: Option<Value>,
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

/// How long a resumed session waits for its interrupted turn to park again
/// before it answers the request that woke it.
/// How far back boot looks for sessions whose turn a restart cut off. Older
/// ones resume when next read.
const RESUME_WINDOW: chrono::TimeDelta = chrono::TimeDelta::days(7);

/// Sessions boot resumes at once.
const RESUME_CONCURRENCY: usize = 4;

const REPARK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long it waits for the rest once one of several calls has parked.
const REPARK_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

struct Live {
    session: everruns::Session,
    active: Mutex<Option<TurnHandle>>,
}

pub(crate) struct Host {
    pub app: App,
    pub mode: Mode,
    pub build_id: String,
    assets: HashMap<String, String>,
    instructions: HashMap<String, String>,
    pub notices: broadcast::Sender<Notice>,
    pub(crate) store: Arc<Store>,
    /// The shared channel runtime over this host's sessions.
    channels: everruns::channels::ChannelHost,
    pub(crate) engine: everruns::Engine,
    native_authority: std::sync::OnceLock<everruns::hosting::SqliteHosting>,
    authority_supervisor: std::sync::OnceLock<tokio::task::AbortHandle>,
    /// `Some` persists sessions through everruns' local store.
    data_dir: Option<PathBuf>,
    live: Mutex<HashMap<String, Arc<Live>>>,
    approvals: Mutex<HashMap<Key, PendingApproval>>,
    questions: Mutex<HashMap<Key, PendingQuestion>>,
    /// Question sets already answered, for `409`.
    answered: Mutex<HashSet<Key>>,
    pub(crate) gateway: gateway::Env,
    /// The hosting target's `[sandbox] kind = "microvm"` adapter, if any.
    microvm: std::sync::OnceLock<crate::hosting::MicroVm>,
    environment_ref: std::sync::OnceLock<String>,
    environment_factory: std::sync::OnceLock<Arc<dyn everruns::hosting::EnvironmentFactory>>,
    /// Who may call the agent routes; unset leaves them open.
    auth: std::sync::OnceLock<crate::auth::Auth>,
    /// Names the session a request just parked on, for AG-UI runs.
    #[cfg(feature = "ag-ui")]
    pub(crate) parked_on: broadcast::Sender<SessionId>,
    me: Weak<Host>,
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(supervisor) = self.authority_supervisor.get() {
            supervisor.abort();
        }
    }
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
    pub(crate) async fn authoritative_status(
        &self,
        id: &str,
    ) -> crate::Result<Option<everruns::hosting::HostingStatus>> {
        if !self.engine.has_session_hosting() {
            return Ok(None);
        }
        let session = id
            .parse()
            .map_err(|_| ApiError::NotFound(format!("session {id}")))?;
        Ok(Some(self.engine.session_status(session).await?))
    }
    pub(crate) fn register_agent(&self, entry: &AgentEntry) -> crate::Result {
        let agent = self.build_agent(entry, None, false)?;
        if self.engine.has_session_hosting() {
            let harness = entry
                .spec
                .package
                .as_ref()
                .map(|package| package.session_harness())
                .transpose()?
                .flatten();
            let build = everruns::hosting::AgentBuild::new(&entry.name, &self.build_id);
            let reference = self
                .environment_ref
                .get()
                .cloned()
                .unwrap_or_else(|| format!("serve-environment-{}", self.build_id));
            if let Some(factory) = self.environment_factory.get() {
                let factory = factory.clone();
                self.engine.register_agent_with_environment(
                    build,
                    agent,
                    harness,
                    reference,
                    move |session| {
                        let factory = factory.clone();
                        async move { factory.open(session).await }
                    },
                )?;
            } else if self.native_authority.get().is_some()
                // An explicit microVM adapter owns its mounted workspace.
                // SQLite journal placement must not replace that binding.
                && !(matches!(self.app.inner.config.sandbox.kind, SandboxKind::Microvm)
                    && self.microvm.get().is_some())
            {
                let root = self
                    .data_dir
                    .as_ref()
                    .ok_or_else(|| anyhow!("native authority requires a data directory"))?
                    .join("workspaces");
                let factory = agent.directory_environment_factory(root);
                self.engine
                    .register_agent_with_environment(build, agent, harness, reference, factory)?;
            } else {
                let factory = agent.environment_factory();
                self.engine
                    .register_agent_with_environment(build, agent, harness, reference, factory)?;
            }
        }
        Ok(())
    }
    /// A host persisting under `data_dir` (dev, start), or in memory (eval).
    pub(crate) fn new(app: App, mode: Mode, data_dir: Option<PathBuf>) -> crate::Result<Arc<Self>> {
        let authority = data_dir
            .as_ref()
            .map(|dir| -> crate::Result<_> {
                std::fs::create_dir_all(dir)?;
                Ok(everruns::hosting::SqliteHosting::open(
                    dir.join("sessions.db"),
                )?)
            })
            .transpose()?;
        let engine = match &authority {
            Some(authority) => everruns::Engine::builder()
                .hosting(authority.clone())
                .build(),
            None => everruns::Engine::new(),
        };
        let host = Self::with_engine(app, mode, data_dir, engine)?;
        if let Some(authority) = authority {
            host.set_native_authority(authority);
        }
        for entry in &host.app.inner.agents {
            host.register_agent(entry)?;
        }
        host.start_authority_supervisor();
        Ok(host)
    }

    pub(crate) fn with_engine(
        app: App,
        mode: Mode,
        data_dir: Option<PathBuf>,
        engine: everruns::Engine,
    ) -> crate::Result<Arc<Self>> {
        let store = Arc::new(match &data_dir {
            Some(dir) => Store::open(&dir.join("serve.db"))?,
            None => Store::in_memory()?,
        });
        let assets: HashMap<String, String> = app
            .inner
            .assets
            .iter()
            .map(|asset| (asset.path.to_string(), load_asset(asset, mode.hot_reload())))
            .collect();
        let instructions: HashMap<String, String> = app
            .inner
            .agents
            .iter()
            .map(|entry| {
                (
                    entry.name.clone(),
                    entry
                        .spec
                        .instructions
                        .as_ref()
                        .map(|instructions| instructions.load(mode.hot_reload()))
                        .unwrap_or_else(|| {
                            assets
                                .get("instructions.md")
                                .cloned()
                                .unwrap_or_else(|| "You are a helpful assistant.".into())
                        }),
                )
            })
            .collect();
        let build_id = match std::env::var("SERVE_BUILD_ID")
            .ok()
            .filter(|id| !id.is_empty())
        {
            Some(retained) => retained,
            None if engine.has_session_hosting() => {
                crate::manifest::execution_build_id(&app, &assets, &instructions)?
            }
            None => crate::manifest::build_id(&app),
        };
        let (notices, _) = broadcast::channel(1024);
        let host = Arc::new_cyclic(|me| Host {
            channels: crate::channels::build(me.clone(), &app, store.clone(), notices.clone()),
            app,
            mode,
            build_id,
            assets,
            instructions,
            notices,
            store,
            engine,
            native_authority: std::sync::OnceLock::new(),
            authority_supervisor: std::sync::OnceLock::new(),
            data_dir,
            live: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            questions: Mutex::new(HashMap::new()),
            answered: Mutex::new(HashSet::new()),
            gateway: gateway::Env::from_process(),
            microvm: std::sync::OnceLock::new(),
            environment_ref: std::sync::OnceLock::new(),
            environment_factory: std::sync::OnceLock::new(),
            auth: std::sync::OnceLock::new(),
            #[cfg(feature = "ag-ui")]
            parked_on: broadcast::channel(64).0,
            me: me.clone(),
        });
        // Validate package runtime bindings before accepting a request. This
        // builds configuration only; no model calls or MCP processes execute.
        for entry in &host.app.inner.agents {
            if entry.spec.package.is_some() {
                host.build_agent(entry, None, false)?;
            }
        }
        #[cfg(feature = "voice")]
        crate::voice::check(&host)?;
        Ok(host)
    }

    /// Install the hosting target's microVM adapter. Set once, before any
    /// agent is built.
    pub(crate) fn set_environment_ref(&self, reference: String) {
        let _ = self.environment_ref.set(reference);
    }
    pub(crate) fn set_environment_factory(
        &self,
        factory: Arc<dyn everruns::hosting::EnvironmentFactory>,
    ) {
        let _ = self.environment_factory.set(factory);
    }
    pub(crate) fn set_native_authority(&self, authority: everruns::hosting::SqliteHosting) {
        let _ = self.native_authority.set(authority);
    }
    pub(crate) fn start_authority_supervisor(&self) {
        let Some(authority) = self.native_authority.get() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.authority_supervisor.get_or_init(|| {
            let authority = authority.clone();
            let engine = self.engine.clone();
            runtime
                .spawn(async move {
                    if let Err(error) = authority
                        .supervise(engine, everruns::CancellationToken::new())
                        .await
                    {
                        eprintln!("serve authority supervisor stopped: {error}");
                    }
                })
                .abort_handle()
        });
    }
    pub(crate) fn set_microvm(&self, microvm: crate::hosting::MicroVm) {
        let _ = self.microvm.set(microvm);
    }

    pub(crate) fn set_auth(&self, auth: crate::auth::Auth) {
        let _ = self.auth.set(auth);
    }

    /// The enforced method list; `None` when the API is open.
    pub(crate) fn auth(&self) -> Option<&crate::auth::Auth> {
        self.auth.get()
    }

    fn arc(&self) -> crate::Result<Arc<Host>> {
        self.me
            .upgrade()
            .ok_or_else(|| anyhow!("host is shutting down"))
    }

    fn notify(&self, notice: Notice) {
        let _ = self.notices.send(notice);
    }

    /// A request parked on `session_id`: wake the AG-UI runs following it.
    fn parked(&self, session_id: &str) {
        #[cfg(feature = "ag-ui")]
        if let Ok(id) = SessionId::parse(session_id) {
            let _ = self.parked_on.send(id);
        }
        #[cfg(not(feature = "ag-ui"))]
        let _ = session_id;
    }

    /// Ids of `agent`'s sessions, most recently active first.
    pub(crate) fn session_ids_for_agent(
        &self,
        agent: &str,
        limit: usize,
    ) -> crate::Result<Vec<String>> {
        self.store.session_ids_for_agent(agent, limit)
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
        let session = if self.engine.has_session_hosting() {
            self.engine.create_registered_with_metadata(new.id.unwrap_or_default(),&everruns::hosting::AgentBuild::new(&entry.name,&self.build_id),json!({"title":new.title,"tags":new.tags,"hints":new.hints,"metadata":new.metadata})).await?
        } else {
            let agent = self.build_agent(&entry, None, true)?;
            match &entry.spec.package {
                Some(package) => package.create(&self.engine, agent)?,
                None => self.engine.create(agent),
            }
        };
        let id = session.id();
        let at = if self.engine.has_session_hosting() {
            self.engine
                .session_status(session.session_id())
                .await?
                .created_at
        } else {
            now()
        };
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
        })?;
        self.go_live(&id, session);
        self.notify(Notice::Live {
            session_id: id.clone(),
            agent: entry.name.to_string(),
            resumed: false,
            after: 0,
        });
        Ok(id)
    }

    fn go_live(&self, id: &str, session: everruns::Session) -> Arc<Live> {
        // Creation retries must retain any observer/cancellation handle already
        // following this session, including while it waits for client tools.
        lock(&self.live)
            .entry(id.to_string())
            .or_insert_with(|| {
                Arc::new(Live {
                    session,
                    active: Mutex::new(None),
                })
            })
            .clone()
    }

    /// The live session, resuming it from the local store after a restart.
    async fn live(&self, id: &str) -> crate::Result<Arc<Live>> {
        if let Some(live) = lock(&self.live).get(id).cloned() {
            return Ok(live);
        }
        if self.engine.has_session_hosting() {
            let session: SessionId = id
                .parse()
                .map_err(|_| ApiError::NotFound(format!("session {id}")))?;
            let status = self.engine.session_status(session).await?;
            let metadata = status.binding.metadata;
            self.store.insert_session(&SessionRow {
                id: id.into(),
                agent: status.binding.behavior.agent,
                build_id: status.binding.behavior.build_id,
                title: metadata["title"].as_str().map(str::to_owned),
                tags: serde_json::from_value(metadata["tags"].clone()).unwrap_or_default(),
                hints: metadata
                    .get("hints")
                    .filter(|value| !value.is_null())
                    .cloned(),
                metadata: metadata
                    .get("metadata")
                    .filter(|value| !value.is_null())
                    .cloned(),
                created_at: status.created_at.clone(),
                updated_at: status.created_at,
            })?;
        }
        let row = self.session_row(id)?;
        if row.build_id != self.build_id && !self.engine.has_session_hosting() {
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
        // Hosted history and settled outcomes remain readable even when this
        // deployment no longer retains the executable Agent. Engine activation
        // handles that case by suspending instead of selecting current code.
        let entry = if self.engine.has_session_hosting() {
            None
        } else {
            Some(
                self.app
                    .agent(&row.agent)
                    .ok_or_else(|| {
                        anyhow!(
                            "session {id} ran agent `{}`, which this build lacks",
                            row.agent
                        )
                    })?
                    .clone(),
            )
        };
        let session_id = SessionId::parse(id)
            .map_err(|err| ApiError::BadRequest(format!("bad session id {id}: {err}")))?;
        if let Some(entry) = &entry {
            let agent = self.build_agent(entry, None, true)?;
            self.engine.attach(session_id, agent).await?;
        }
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
                active: Mutex::new(None),
            });
            map.insert(id.to_string(), live.clone());
            live
        };
        self.notify(Notice::Live {
            session_id: id.to_string(),
            agent: row.agent,
            resumed: true,
            after,
        });
        if let Some(entry) = &entry {
            self.resume_interrupted(id, &live, entry).await?;
        }
        Ok(live)
    }

    /// Pick up a turn the last process cut off. Calls that are safe to run
    /// again run again, the rest are recorded as interrupted, and the turn
    /// carries on. When some of those calls wait on a person, return once
    /// they have parked here again (or the turn ended), so the request that
    /// woke the session sees them open.
    async fn resume_interrupted(
        &self,
        id: &str,
        live: &Arc<Live>,
        entry: &AgentEntry,
    ) -> crate::Result {
        if self.engine.has_session_hosting() {
            return Ok(());
        }
        let Some(interrupted) = live.session.interrupted_turn().await? else {
            return Ok(());
        };
        let expected = interrupted
            .tool_calls
            .iter()
            .filter(|call| !interrupted.not_rerun.contains(call))
            .filter(|call| self.waits_on_person(entry, call))
            .count();
        let mut notices = self.notices.subscribe();
        let Some(turn) = live.session.resume_interrupted_turn().await? else {
            return Ok(());
        };
        self.follow(live, turn.clone());
        if expected == 0 {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + REPARK_WAIT;
        loop {
            let parked = self.pending_approvals(id).len() + self.pending_questions(id).len();
            if parked >= expected {
                break;
            }
            // Calls the act runs one after another park one at a time: once
            // the first is back, the rest get only a moment.
            let until = if parked > 0 {
                deadline.min(tokio::time::Instant::now() + REPARK_GRACE)
            } else {
                deadline
            };
            tokio::select! {
                notice = notices.recv() => {
                    if let Err(broadcast::error::RecvError::Closed) = notice {
                        break;
                    }
                }
                _ = turn.wait() => break,
                () = tokio::time::sleep_until(until) => break,
            }
        }
        Ok(())
    }

    /// Whether `call` waits on a person before it runs: `ask_user`, or a
    /// tool whose approval rule gates these arguments.
    fn waits_on_person(&self, entry: &AgentEntry, call: &ToolCall) -> bool {
        if call.name == "ask_user" {
            return true;
        }
        self.app
            .tools_for(entry)
            .into_iter()
            .find(|tool| tool.name == call.name)
            .is_some_and(|tool| match tool.approval {
                Approval::Never => false,
                Approval::Always => true,
                Approval::When(predicate) => predicate(&call.arguments),
            })
    }

    /// Resume every turn a restart cut off, in the background: wake each
    /// session active within [`RESUME_WINDOW`], a few at a time. A session
    /// outside the window still resumes when it is next read.
    pub(crate) fn spawn_resume_interrupted(self: &Arc<Self>) {
        let since = (chrono::Utc::now() - RESUME_WINDOW)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let ids = match self.store.session_ids_active_since(&since) {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!("  resuming interrupted turns failed: {error}");
                return;
            }
        };
        let host = self.clone();
        tokio::spawn(async move {
            use futures::StreamExt as _;
            futures::stream::iter(ids)
                .for_each_concurrent(RESUME_CONCURRENCY, |id| {
                    let host = host.clone();
                    async move { host.wake(&id).await }
                })
                .await;
        });
    }

    /// Wake `id` after a restart, so a turn it left running or waiting on a
    /// person carries on. Reading a session never depends on it: a session this
    /// build cannot run is still shown.
    pub(crate) async fn wake(&self, id: &str) {
        let _ = self.live(id).await;
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

    /// Whether any session has a turn running that is not parked on a
    /// person. A turn waiting for an approval or an answer counts as idle:
    /// nothing runs until someone replies.
    pub(crate) fn busy(&self) -> bool {
        let ids: Vec<String> = lock(&self.live).keys().cloned().collect();
        ids.iter().any(|id| self.status(id) == "active")
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
        self.track(id, &live, &sent);
        let handle = sent.turn();
        Ok(PendingTurn {
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

    /// Send any input message, as a channel does; the turn is tracked like
    /// any other.
    pub(crate) async fn send_message(
        &self,
        id: &str,
        message: everruns::InputMessage,
    ) -> crate::Result<SentMessage> {
        let live = self.live(id).await?;
        let sent = live.session.send(message).await?;
        self.track(id, &live, &sent);
        Ok(sent)
    }

    pub(crate) async fn send_message_with_id(
        &self,
        id: &str,
        message_id: everruns::MessageId,
        message: everruns::InputMessage,
    ) -> crate::Result<SentMessage> {
        let live = self.live(id).await?;
        let sent = live.session.send_with_id(message_id, message).await?;
        self.track(id, &live, &sent);
        Ok(sent)
    }

    /// The turn running on `id`'s session, if one is.
    pub(crate) fn active_turn(&self, id: &str) -> Option<TurnHandle> {
        let live = lock(&self.live).get(id).cloned()?;
        lock(&live.active).clone()
    }

    /// The shared channel runtime: inbound requests, proactive posts,
    /// recovery.
    pub(crate) fn channels(&self) -> &everruns::channels::ChannelHost {
        &self.channels
    }

    /// Note a message accepted by `id`'s session: touch the catalog and, when
    /// it started a turn, follow that turn (status, cancel, delivery).
    fn track(&self, id: &str, live: &Arc<Live>, sent: &SentMessage) {
        let _ = self.store.touch(id, &now());
        if matches!(sent.disposition, SendDisposition::Steered) {
            return;
        }
        self.follow(live, sent.turn());
    }

    /// Follow a running turn for status and cancel. Channel replies are
    /// delivered by the channel runtime while the turn runs.
    fn follow(&self, live: &Arc<Live>, turn: TurnHandle) {
        *lock(&live.active) = Some(turn.clone());
        let live = live.clone();
        let engine = self.engine.clone();
        let notices = self.notices.clone();
        tokio::spawn(async move {
            let mut observed = HashSet::new();
            loop {
                tokio::select! {
                    // Drop the losing wait before querying status. A retained
                    // wait may reserve the authority's fair mutex while pending;
                    // awaiting another query here would stop polling its owner.
                    _=turn.wait()=>break,
                    ()=tokio::time::sleep(std::time::Duration::from_millis(25)),if engine.has_session_hosting()=>{
                        if let Ok(status)=engine.session_status(live.session.session_id()).await {
                            for call in status.waiting {
                                if !observed.insert((status.open_turn,call.id.clone())) {continue;}
                                if let Some(request)=everruns_contracts::tool_approval_types::ToolApprovalRequired::from_request_call(&call.id,&call.name,&call.arguments) {
                                    let _=notices.send(Notice::ApprovalRequested(PendingApprovalView{session_id:live.session.id(),tool_call_id:call.id,tool_name:request.tool,arguments:request.arguments}));
                                } else if call.name=="ask_user"
                                    && let Ok(questions)=serde_json::from_value(call.arguments["questions"].clone()) {let _=notices.send(Notice::QuestionAsked{session_id:live.session.id(),tool_call_id:call.id,questions});}
                            }
                        }
                    }
                }
            }
            let mut active = lock(&live.active);
            // A newer turn may already be the active one.
            if active
                .as_ref()
                .is_some_and(|current| current.identity() == turn.identity())
            {
                *active = None;
            }
        });
    }

    /// Answer one AG-UI request on `id`'s session. The run sends the user
    /// message itself; the turn it starts is tracked like any other.
    #[cfg(feature = "ag-ui")]
    pub(crate) async fn ag_ui(
        &self,
        id: &str,
        input: everruns::ag_ui::RunAgentInput,
        options: everruns::ag_ui::AgUiOptions,
    ) -> crate::Result<everruns::ag_ui::AgUiStream> {
        let live = self.live(id).await?;
        let run = live
            .session
            .ag_ui_with(input, options)
            .await
            .map_err(|err| match err {
                everruns::ag_ui::AgUiError::InvalidInput(why) => {
                    anyhow::Error::from(ApiError::BadRequest(why))
                }
                other => anyhow::Error::from(other),
            })?;
        if let Some(sent) = run.sent() {
            self.track(id, &live, sent);
        }
        Ok(run)
    }

    /// The session bound to a channel thread, if any.
    #[cfg(all(test, any(feature = "ag-ui", feature = "a2a")))]
    pub(crate) fn thread_session(
        &self,
        channel: &str,
        thread: &str,
    ) -> crate::Result<Option<String>> {
        self.store.thread_session(channel, thread)
    }

    /// Cancel the active turn. `Ok(false)` when no turn was running.
    #[cfg(feature = "a2a")]
    pub(crate) async fn cancel(&self, id: &str) -> crate::Result<bool> {
        self.cancel_target(id, uuid::Uuid::new_v4(), None).await
    }

    pub(crate) async fn cancel_target(
        &self,
        id: &str,
        command: uuid::Uuid,
        message: Option<everruns::MessageId>,
    ) -> crate::Result<bool> {
        let live = self.live(id).await?;
        if let Some(status) = self.authoritative_status(id).await? {
            if let Some(message) = message {
                let admission = live
                    .session
                    .admission(message)
                    .await?
                    .ok_or_else(|| ApiError::NotFound("admission receipt".into()))?;
                admission.cancel(command).await?;
                return Ok(true);
            }
            let Some(turn) = status.open_turn else {
                return Ok(false);
            };
            let events = live.session.events_after(0).await?;
            let target = events
                .iter()
                .find(|event| event.canonical_json()["context"]["turn_id"] == turn.to_string())
                .and_then(|event| {
                    event.canonical_json()["context"]["input_message_id"]
                        .as_str()
                        .map(str::to_owned)
                });
            let target = target
                .ok_or_else(|| ApiError::Conflict("open turn has no input identity".into()))?;
            let target = target.parse()?;
            let admission =
                live.session.admission(target).await?.ok_or_else(|| {
                    ApiError::Conflict("open turn has no admission receipt".into())
                })?;
            admission.cancel(command).await?;
            return Ok(true);
        }
        let active = lock(&live.active).clone();
        // Stop the turn before dropping what it waits on: a dropped approval
        // reads as a rejection, and the turn could finish on it first.
        let cancelled = match active {
            Some(turn) => turn.cancel().await.is_ok(),
            None => false,
        };
        // A cancelled turn will never collect its pending approvals or answers.
        lock(&self.approvals).retain(|_, pending| pending.view.session_id != id);
        lock(&self.questions).retain(|_, pending| pending.session_id != id);
        Ok(cancelled)
    }

    // --- Approvals and questions ---------------------------------------------

    /// Resolve a pending approval. `NotFound` when there is none by that id.
    pub(crate) async fn resolve_approval(
        &self,
        session_id: &str,
        tool_call_id: &str,
        approve: bool,
    ) -> crate::Result {
        let decision = if approve {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Reject
        };
        self.decide_approval(session_id, tool_call_id, decision)
            .await
    }

    /// Resolve a pending approval with any decision, `*_always` included.
    pub(crate) async fn decide_approval(
        &self,
        session_id: &str,
        tool_call_id: &str,
        decision: ApprovalDecision,
    ) -> crate::Result {
        if self.engine.has_session_hosting() {
            if !self
                .engine
                .session_status(session_id.parse()?)
                .await?
                .waiting
                .iter()
                .any(|call| call.id == tool_call_id && call.name == "approve_tool_call")
            {
                return Err(ApiError::NotFound(format!("pending approval {tool_call_id}")).into());
            }
            let decision = match decision {
                ApprovalDecision::Allow => everruns::hosting::ToolApprovalDecision::Allow,
                ApprovalDecision::Reject => everruns::hosting::ToolApprovalDecision::Reject,
                ApprovalDecision::AllowAlways => {
                    everruns::hosting::ToolApprovalDecision::AllowAlways
                }
                ApprovalDecision::RejectAlways => {
                    everruns::hosting::ToolApprovalDecision::RejectAlways
                }
                _ => {
                    return Err(ApiError::BadRequest(
                        "an explicit approval decision is required".into(),
                    )
                    .into());
                }
            };
            self.session(session_id)
                .await?
                .approve_tool(
                    uuid::Uuid::new_v4(),
                    self.engine
                        .session_status(session_id.parse()?)
                        .await?
                        .open_turn
                        .ok_or_else(|| ApiError::Conflict("no open turn".into()))?,
                    tool_call_id,
                    decision,
                )
                .await?;
            return Ok(());
        }
        self.decide_local_approval(session_id, tool_call_id, decision)
    }

    pub(crate) fn decide_local_approval(
        &self,
        session_id: &str,
        tool_call_id: &str,
        decision: ApprovalDecision,
    ) -> crate::Result {
        let pending =
            { lock(&self.approvals).remove(&(session_id.to_string(), tool_call_id.to_string())) };
        let Some(pending) = pending else {
            return Err(ApiError::NotFound(format!("pending approval {tool_call_id}")).into());
        };
        let _ = pending.tx.send(decision);
        Ok(())
    }

    /// Answer a pending `ask_user` question set, like the everruns server's
    /// `POST /question-answers`. Without `tool_call_id`, the one pending set
    /// of the session is answered.
    pub(crate) async fn answer_questions(
        &self,
        session_id: &str,
        tool_call_id: Option<&str>,
        status: Status,
        answers: Vec<Answer>,
    ) -> crate::Result {
        if self.engine.has_session_hosting() {
            let session = self.session(session_id).await?;
            let pending = session.pending_tools().await?;
            let mut matches = pending.iter().filter(|call| {
                call.name == "ask_user" && tool_call_id.is_none_or(|id| id == call.id)
            });
            let call = matches
                .next()
                .ok_or_else(|| ApiError::NotFound("pending question set".into()))?;
            if matches.next().is_some() {
                return Err(ApiError::BadRequest(
                    "several question sets are pending; pass tool_call_id".into(),
                )
                .into());
            }
            let questions: Vec<Question> =
                serde_json::from_value(call.arguments["questions"].clone())?;
            if status == Status::Answered {
                validate_answers(&questions, &answers).map_err(ApiError::BadRequest)?;
            }
            session
                .resolve_tools(
                    uuid::Uuid::new_v4(),
                    self.engine
                        .session_status(session_id.parse()?)
                        .await?
                        .open_turn
                        .ok_or_else(|| ApiError::Conflict("no open turn".into()))?,
                    vec![everruns::ToolAnswer {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        result: serde_json::to_value(Outcome {
                            status,
                            answered_by: AnsweredBy::User,
                            answers: if status == Status::Answered {
                                answers
                            } else {
                                vec![]
                            },
                        })?,
                        error: None,
                    }],
                )
                .await?;
            return Ok(());
        }
        self.answer_local_questions(session_id, tool_call_id, status, answers)
    }

    pub(crate) fn answer_local_questions(
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
        host.parked(&view.session_id);
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
        host.parked(&session_id);
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
use everruns::hosting::validate_question_answers as validate_answers;

/// The event envelope the wire API sends: the complete canonical envelope,
/// with reasoning replay state stripped the way the everruns server's public
/// projection does (opaque provider parts dropped; reasoning signatures and
/// encrypted payloads removed).
pub(crate) fn wire_json(event: &SessionEvent) -> Value {
    everruns_contracts::execution_api::public_event(event.canonical_json())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::ask_user::{AskUserOption, QuestionKind};

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
