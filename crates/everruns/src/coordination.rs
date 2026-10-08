//! Coordinator agents: one session hands focused work to threads.
//!
//! Stability: alpha.
//!
//! A coordinator is an agent with the [`Coordination`] capability. It starts a
//! **thread** (a child session working one piece of work), relays follow-ups to
//! it, and reports back. Threads report through tools of their own: a
//! checklist, a completion with a summary, a question, a redirect. Every
//! report wakes the coordinator with an automatic update, so the person keeps
//! talking to one session while the work happens elsewhere.
//!
//! Coordination needs the `local` feature and an agent built with
//! [`AgentBuilder::local`](crate::AgentBuilder::local): threads and their
//! assignments live in the local profile's SQLite store. Threads run the
//! coordinator's own agent, in the same [`Engine`].
//!
//! # Example
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::coordination::{self, Coordination};
//! use everruns::{Agent, Engine, LocalConfig, Model};
//!
//! let agent = Agent::builder()
//!     .instructions("Hand each piece of work to a thread.")
//!     .model(Model::simulated("On it."))
//!     .capability(Coordination::new().max_active_threads(3))
//!     .local(LocalConfig::new("./.everruns"))
//!     .build()?;
//! let session = Engine::new().create(agent);
//! session.run("Plan the launch: copy, pricing page, and a demo video.").await?;
//! for thread in coordination::threads(&session).await? {
//!     println!("{:?}  {}", thread.status, thread.title);
//! }
//! # Ok(())
//! # }
//! ```

// Decision: the Framework runs threads through the engine that owns the
// coordinator (`EngineSessionRunner`), so a thread is an ordinary engine
// session the application can resume and read like any other.
// Decision: only `self` workers. A local profile has no agent catalog to look
// another worker up in, so the runner refuses any other agent id.
// Decision: the wake path covers `assignment` tasks only. Other task kinds
// keep their existing between-turn delivery; widening the waker would change
// how every local background task reaches its session.
// Decision: thread turns the runner starts are settled right after they end.
// A turn the application starts on a thread directly is not settled: the
// person talks to the coordinator, and it routes (user decision, 2026-10-08).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_capabilities::capabilities::coordination::{
    THREAD_RESOLVED_DETAIL, ThreadTurn, settle_thread_turn,
};
use everruns_capabilities::{PlatformCreateSessionRequest, PlatformMessage};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::host::{
    EventHistory, EventHistoryReadLimit, EventHistoryReadRequest, MAX_EVENT_HISTORY_PAGE_SIZE,
    RuntimeSessionStore, SessionBuilder,
};
use everruns_core::session::ExecutionSession;
use everruns_core::session_task::{
    SessionTask, SessionTaskFilter, SessionTaskRegistry, SessionTaskState, TASK_KIND_ASSIGNMENT,
};
use everruns_core::turn::TurnStopReason;
use everruns_core::{
    InputMessage, ObservingTaskRegistry, RuntimeMessageRole, TaskTransition,
    TaskTransitionObserver, wake_text_for,
};

use crate::engine::{Engine, WeakEngine};
use crate::local::LocalSessionRunner;
use crate::{CapabilityRef, CapabilitySpec, IntoCapability, Session};

/// Capability id of [`Coordination`].
pub const CAPABILITY_ID: &str = "coordination";

/// Lets an agent start threads, relay to them, and resolve them.
///
/// Defaults: threads run this agent, at most 8 work at once, 100 in total.
#[derive(Clone, Debug, Default)]
pub struct Coordination {
    max_active_threads: Option<u64>,
    max_total_threads: Option<u64>,
}

impl Coordination {
    /// Coordination with the default limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Most threads working at the same time.
    pub fn max_active_threads(mut self, limit: u64) -> Self {
        self.max_active_threads = Some(limit);
        self
    }

    /// Most threads this coordinator may ever start.
    pub fn max_total_threads(mut self, limit: u64) -> Self {
        self.max_total_threads = Some(limit);
        self
    }
}

impl IntoCapability for Coordination {
    fn into_capability(self) -> CapabilitySpec {
        let mut config = serde_json::Map::new();
        if let Some(limit) = self.max_active_threads {
            config.insert("max_active_threads".into(), limit.into());
        }
        if let Some(limit) = self.max_total_threads {
            config.insert("max_total_threads".into(), limit.into());
        }
        CapabilityRef::new(CAPABILITY_ID)
            .config(serde_json::Value::Object(config))
            .into()
    }
}

/// Where a thread stands, from its latest assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThreadStatus {
    /// The thread asked a question, or its work failed.
    NeedsYou,
    /// The thread is working.
    Working,
    /// The thread finished; the coordinator has not resolved it yet.
    ReadyForReview,
    /// The work was cancelled and the thread was left open.
    Open,
    /// The coordinator resolved the thread.
    Resolved,
}

/// One thread a coordinator started.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Thread {
    /// The thread's session id; [`Engine::resume`] opens it.
    pub id: SessionId,
    /// Title of the latest assignment.
    pub title: String,
    /// Where the thread stands.
    pub status: ThreadStatus,
    /// The thread's completion summary, once it finished.
    pub summary: Option<String>,
    /// The question the thread is waiting on, if any.
    pub question: Option<String>,
    /// Checklist steps as `(title, status)`, e.g. `("Draft copy", "done")`.
    pub checklist: Vec<(String, String)>,
    /// Units of work this thread has carried.
    pub assignments: usize,
}

/// The threads a coordinator session started, oldest first.
pub async fn threads(session: &Session) -> std::result::Result<Vec<Thread>, AgentLoopError> {
    let execution = session.inner().execution.clone();
    let backends = execution
        .backends()
        .await
        .map_err(crate::agent::BackendInitError::into_agent_loop)?;
    let Some(registry) = backends.host.session_task_registry.clone() else {
        return Ok(Vec::new());
    };
    let tasks = registry
        .list(
            execution.session_id(),
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_ASSIGNMENT.to_string()),
                state: None,
            }),
        )
        .await?;
    let mut threads: Vec<(SessionTask, usize)> = Vec::new();
    for task in tasks {
        let Some(thread) = task.links.child_session_id else {
            continue;
        };
        match threads
            .iter_mut()
            .find(|(latest, _)| latest.links.child_session_id == Some(thread))
        {
            Some((latest, count)) => {
                *count += 1;
                if task.created_at > latest.created_at {
                    *latest = task;
                }
            }
            None => threads.push((task, 1)),
        }
    }
    Ok(threads
        .into_iter()
        .map(|(task, assignments)| Thread {
            id: task.links.child_session_id.expect("filtered above"),
            title: task.display_name.clone(),
            status: thread_status(&task),
            summary: task.summary.clone(),
            question: task.input_request.as_ref().map(|r| r.prompt.clone()),
            checklist: task
                .progress
                .as_ref()
                .map(|progress| {
                    progress
                        .steps
                        .iter()
                        .map(|step| {
                            let status = serde_json::to_value(step.status)
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_string))
                                .unwrap_or_default();
                            (step.title.clone(), status)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            assignments,
        })
        .collect())
}

fn thread_status(task: &SessionTask) -> ThreadStatus {
    if task.state_detail.as_deref() == Some(THREAD_RESOLVED_DETAIL) {
        return ThreadStatus::Resolved;
    }
    match task.state {
        SessionTaskState::AwaitingInput | SessionTaskState::Failed => ThreadStatus::NeedsYou,
        SessionTaskState::Queued | SessionTaskState::Running => ThreadStatus::Working,
        SessionTaskState::Succeeded => ThreadStatus::ReadyForReview,
        _ => ThreadStatus::Open,
    }
}

// =============================================================================
// Engine wiring
// =============================================================================

/// Engines sharing one local backend bundle. Threads and wakes reach a session
/// through whichever engine holds it.
#[derive(Default)]
pub(crate) struct EngineHub {
    engines: Mutex<Vec<WeakEngine>>,
}

impl EngineHub {
    pub(crate) fn register(&self, engine: &Engine) {
        let mut engines = self.engines.lock().unwrap_or_else(|p| p.into_inner());
        engines.retain(|weak| weak.upgrade().is_some());
        if !engines.iter().any(|weak| weak.is(engine)) {
            engines.push(engine.downgrade());
        }
    }

    fn owner(&self, session_id: SessionId) -> Option<Engine> {
        let engines = self.engines.lock().unwrap_or_else(|p| p.into_inner());
        engines
            .iter()
            .filter_map(WeakEngine::upgrade)
            .find(|engine| engine.agent(session_id).is_some())
    }
}

/// Wrap the profile's task registry so assignment updates wake coordinators.
pub(crate) fn observed_registry(
    inner: Arc<dyn SessionTaskRegistry>,
    hub: Arc<EngineHub>,
) -> Arc<dyn SessionTaskRegistry> {
    Arc::new(ObservingTaskRegistry::new(inner).with_observer(Arc::new(CoordinatorWaker { hub })))
}

/// Starts (or steers) a coordinator turn with an automatic update when one of
/// its assignments changes, the Framework form of the server's task waker.
struct CoordinatorWaker {
    hub: Arc<EngineHub>,
}

#[async_trait]
impl TaskTransitionObserver for CoordinatorWaker {
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()> {
        // Resolution is the coordinator's own act; it needs no update about it.
        if task.kind != TASK_KIND_ASSIGNMENT
            || task.state_detail.as_deref() == Some(THREAD_RESOLVED_DETAIL)
        {
            return Ok(());
        }
        let Some(text) = wake_text_for(task, transition) else {
            return Ok(());
        };
        let Some(engine) = self.hub.owner(task.session_id) else {
            return Ok(());
        };
        let coordinator = task.session_id;
        tokio::spawn(async move {
            let session = match engine.resume(coordinator).await {
                Ok(session) => session,
                Err(error) => {
                    tracing::warn!(%coordinator, %error, "coordination: wake failed");
                    return;
                }
            };
            let mut input = InputMessage::user(text);
            input.metadata = Some(everruns_core::message::task_wake_message_metadata());
            // Hold the session until the turn ends, so dropping the last
            // application handle does not cut the automatic turn short.
            match session.send(input).await {
                Ok(sent) => {
                    let _ = sent.wait().await;
                }
                Err(error) => tracing::warn!(%coordinator, %error, "coordination: wake failed"),
            }
        });
        Ok(())
    }
}

/// Runs threads as sessions of the engine that owns their coordinator.
pub(crate) struct EngineSessionRunner {
    hub: Arc<EngineHub>,
    sessions: Arc<dyn RuntimeSessionStore>,
    registry: Arc<dyn SessionTaskRegistry>,
}

impl EngineSessionRunner {
    pub(crate) fn new(
        hub: Arc<EngineHub>,
        sessions: Arc<dyn RuntimeSessionStore>,
        registry: Arc<dyn SessionTaskRegistry>,
    ) -> Self {
        Self {
            hub,
            sessions,
            registry,
        }
    }

    fn owner(&self, session_id: SessionId) -> Result<Engine> {
        self.hub
            .owner(session_id)
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))
    }

    async fn settle(&self, coordinator: Option<SessionId>, thread: SessionId, turn: ThreadTurn) {
        let Some(coordinator) = coordinator else {
            return;
        };
        if let Err(error) =
            settle_thread_turn(self.registry.as_ref(), coordinator, thread, turn).await
        {
            tracing::debug!(%thread, %error, "coordination: thread turn settle failed");
        }
    }
}

#[async_trait]
impl LocalSessionRunner for EngineSessionRunner {
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        let parent = parent_session_id.ok_or_else(|| {
            AgentLoopError::tool("a Framework engine only creates child sessions of its own")
        })?;
        let engine = self.owner(parent)?;
        let agent = engine
            .agent(parent)
            .ok_or_else(|| AgentLoopError::session_not_found(parent))?;
        let parent_record = self.sessions.get_session(parent).await?;
        if let Some(agent_id) = agent_id
            && parent_record.as_ref().and_then(|s| s.agent_id) != Some(agent_id)
        {
            return Err(AgentLoopError::tool(
                "a Framework engine runs threads with the coordinator's own agent only",
            ));
        }
        let mut session = SessionBuilder::new(harness_id)
            .id(SessionId::new())
            .title(title.unwrap_or("Thread"))
            .build();
        session.agent_id = agent_id;
        session.locale = locale.map(str::to_string);
        session.parent_session_id = Some(parent);
        // Catalog first: the thread's runtime keeps these fields when it builds.
        self.sessions.add_session(session.clone()).await?;
        engine.attach_child(session.id, agent);
        Ok(session)
    }

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        let goal = request.goal.clone();
        let mut session = self
            .create_session(
                request.harness_id,
                request.agent_id,
                request.title.as_deref(),
                request.locale.as_deref(),
                request.parent_session_id,
            )
            .await?;
        if goal.is_some() {
            session.goal = goal;
            self.sessions.add_session(session.clone()).await?;
        }
        Ok(session)
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let engine = self.owner(session_id)?;
        let session = engine
            .resume(session_id)
            .await
            .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        let coordinator = self
            .sessions
            .get_session(session_id)
            .await?
            .and_then(|record| record.parent_session_id);
        self.settle(coordinator, session_id, ThreadTurn::Started)
            .await;
        let result = session.run(content).await;
        let turn = match &result {
            Ok(turn) if turn.success => ThreadTurn::Completed,
            Ok(turn) if turn.stop_reason == TurnStopReason::Cancelled => ThreadTurn::Cancelled,
            _ => ThreadTurn::Failed,
        };
        self.settle(coordinator, session_id, turn).await;
        match result {
            Ok(turn) if turn.success => Ok(()),
            Ok(turn) => Err(AgentLoopError::tool(format!(
                "thread turn failed: {}",
                turn.error.unwrap_or_default()
            ))),
            Err(error) => Err(AgentLoopError::tool(error.to_string())),
        }
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>> {
        Ok(Vec::new())
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.sessions.get_session(session_id).await
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        let engine = self.owner(session_id)?;
        let agent = engine
            .agent(session_id)
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
        let backends = engine
            .backends_for(&agent)
            .await
            .map_err(crate::agent::BackendInitError::into_agent_loop)?;
        let history = EventHistory::new(backends.host.event_log.clone());
        let page_size = EventHistoryReadLimit::new(MAX_EVENT_HISTORY_PAGE_SIZE)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let mut request = EventHistoryReadRequest::new(session_id, page_size);
        let mut messages = Vec::new();
        loop {
            let page = history
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            messages.extend(page.messages.into_iter().map(|message| PlatformMessage {
                role: match &message.role {
                    RuntimeMessageRole::Agent => "agent".to_string(),
                    RuntimeMessageRole::User => "user".to_string(),
                    other => format!("{other:?}").to_lowercase(),
                },
                content: message.text().unwrap_or_default().to_string(),
                created_at: message.created_at,
            }));
            let Some(cursor) = page.next_cursor else {
                break;
            };
            request = EventHistoryReadRequest::new(session_id, page_size).with_cursor(cursor);
        }
        // Most recent first, as the platform store contract reads them.
        messages.reverse();
        if let Some(limit) = limit {
            messages.truncate(limit);
        }
        Ok(messages)
    }

    async fn get_session_status(&self, session_id: SessionId) -> Result<Option<String>> {
        // `send_message` runs the turn to completion, so a known thread is idle
        // whenever anything polls it.
        Ok(self
            .sessions
            .get_session(session_id)
            .await?
            .map(|_| "idle".to_string()))
    }
}
