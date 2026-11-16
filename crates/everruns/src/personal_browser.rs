//! Host-owned durable dispatch bridge. Runs outside an agent execution lease.
//! THREAT[TM-BROWSER-005]: bind each supervisor to its full authoritative session.
//! THREAT[TM-BROWSER-007]: only verified upstream handoff outcomes resolve waits.
//! Configuration/worker ownership is trusted host input, not tool arguments.
//! Personal mode is a single-user trust domain: upstream user-lent tabs are
//! shared with connected agents. Process ownership isolates agent-created tabs,
//! not lent tabs across tenants. Session opt-in avoids accidental child exposure;
//! it does not impose a stronger lending partition than the upstream extension.
//! Hosts must call `run_pending` on notifications and restart, and schedule a
//! retry at `next_check_at_ms` when another live worker owns a pending claim.
use crate::hosting::{
    SessionConfiguration,
    spi::session_commands::{
        ClientDispatchClaimReply, ClientDispatchRecord, ClientDispatchState, SessionCommand,
    },
};
use crate::{AgentBuilder, Engine, SessionId, ToolAnswer, ToolCall};
use everruns_contracts::tool_types::{ClientSideTool, ToolDefinition};
pub use everruns_integrations::personal_browser::{
    BrowseError, BrowseProcessConfig, BrowseState, BrowseSupervisor, HandoffOutcome,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

mod actions;
mod model;
use model::{action_timeout, configure, declarations, execute};

/// Read-only personal browser connection tool name.
pub const STATUS_TOOL: &str = "personal_browser_status";
/// Read-only personal browser tab-list tool name.
pub const TABS_TOOL: &str = "personal_browser_tabs";
/// Durable human handoff tool name.
pub const HANDOFF_TOOL: &str = "personal_browser_handoff";
fn owns(name: &str) -> bool {
    matches!(name, STATUS_TOOL | TABS_TOOL | HANDOFF_TOOL) || actions::upstream(name).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Redacted host coordinator failure.
pub enum CoordinatorError {
    /// Authority transaction or session binding is unavailable.
    Authority,
    /// Host configuration or reserved tool declaration conflicts.
    InvalidConfiguration,
    /// This coordinator binding is absent or permanently closed.
    Closed,
    /// Host-configured session limit reached.
    Capacity,
    /// Pinned process could not initialize.
    BrowserUnavailable,
}
impl std::fmt::Display for CoordinatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CoordinatorError {}
#[derive(Default, Debug)]
/// Outcome of one reconciliation pass.
pub struct DispatchReport {
    /// Calls durably completed in this pass.
    pub completed: usize,
    /// Calls durably marked interrupted without replay.
    pub interrupted: usize,
    /// Authority-clock deadline; host must schedule reconciliation then.
    pub next_check_at_ms: Option<i64>,
}
struct Binding {
    stop: watch::Sender<bool>,
    supervisor: Mutex<Option<BrowseSupervisor>>,
}
struct Inner {
    incarnation: Uuid,
    factory: Box<dyn Fn(SessionId) -> BrowseProcessConfig + Send + Sync>,
    max_sessions: usize,
    bindings: Mutex<HashMap<SessionId, Arc<Binding>>>,
}
#[derive(Clone)]
/// Host-owned coordinator with independent session processes and cancellation.
pub struct PersonalBrowserCoordinator {
    inner: Arc<Inner>,
}
impl PersonalBrowserCoordinator {
    /// Create a bounded coordinator with a trusted per-session process factory.
    pub fn new(
        factory: impl Fn(SessionId) -> BrowseProcessConfig + Send + Sync + 'static,
        max_sessions: usize,
    ) -> Result<Self, CoordinatorError> {
        if max_sessions == 0 || max_sessions > 256 {
            return Err(CoordinatorError::InvalidConfiguration);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                incarnation: Uuid::new_v4(),
                factory: Box::new(factory),
                max_sessions,
                bindings: Mutex::new(HashMap::new()),
            }),
        })
    }
    /// For hosts that service every session of this Agent. Prefer enable_session
    /// when only explicitly selected sessions have a browser worker.
    pub fn declare_tools(mut builder: AgentBuilder) -> AgentBuilder {
        for tool in declarations() {
            builder = builder.client_tool(tool.name, tool.description, tool.parameters);
        }
        builder
    }
    /// Enable only this session, never implicitly its children. Call before
    /// admitting work; host configuration changes must be serialized by host.
    pub async fn enable_session(
        &self,
        engine: &Engine,
        session: SessionId,
    ) -> Result<(), CoordinatorError> {
        self.bind_session(engine, session).await?;
        let binding = self
            .inner
            .bindings
            .lock()
            .await
            .get(&session)
            .cloned()
            .ok_or(CoordinatorError::Closed)?;
        let _guard = binding.supervisor.lock().await;
        if *binding.stop.borrow() {
            return Err(CoordinatorError::Closed);
        }
        configure(engine, session, true).await?;
        if *binding.stop.borrow() {
            configure(engine, session, false).await?;
            return Err(CoordinatorError::Closed);
        }
        Ok(())
    }
    /// Removes only this bridge's tools; refuses outstanding browser calls.
    /// Resolve/recover those calls first rather than orphaning a durable wait.
    pub async fn disable_session(
        engine: &Engine,
        session: SessionId,
    ) -> Result<(), CoordinatorError> {
        let status = engine
            .session_status(session)
            .await
            .map_err(|_| CoordinatorError::Authority)?;
        if status.waiting.iter().any(|call| owns(&call.name)) {
            return Err(CoordinatorError::InvalidConfiguration);
        }
        configure(engine, session, false).await
    }
    /// Initialize one existing session without exposing tools or granting consent.
    pub async fn bind_session(
        &self,
        engine: &Engine,
        session: SessionId,
    ) -> Result<(), CoordinatorError> {
        engine
            .session_status(session)
            .await
            .map_err(|_| CoordinatorError::Authority)?;
        let binding = {
            let mut bindings = self.inner.bindings.lock().await;
            if let Some(binding) = bindings.get(&session) {
                binding.clone()
            } else {
                if bindings.len() >= self.inner.max_sessions {
                    return Err(CoordinatorError::Capacity);
                }
                let (stop, _) = watch::channel(false);
                let binding = Arc::new(Binding {
                    stop,
                    supervisor: Mutex::new(None),
                });
                bindings.insert(session, binding.clone());
                binding
            }
        };
        let mut stopped = binding.stop.subscribe();
        if *stopped.borrow() {
            return Err(CoordinatorError::Closed);
        }
        let mut supervisor = binding.supervisor.lock().await;
        if *stopped.borrow() {
            return Err(CoordinatorError::Closed);
        }
        if supervisor.is_none() {
            let started = tokio::select! {
                biased;
                _=stopped.changed()=>return Err(CoordinatorError::Closed),
                started=BrowseSupervisor::start(session.uuid(),(self.inner.factory)(session))=>started,
            };
            match started {
                Ok(started) => *supervisor = Some(started),
                Err(_) => {
                    binding.stop.send_replace(true);
                    return Err(CoordinatorError::BrowserUnavailable);
                }
            }
        }
        Ok(())
    }
    /// Host worker entry point. Never call from an agent tool handler: client
    /// tools are already parked and the agent must not retain its worker lease.
    /// Concurrent calls serialize by session; other sessions remain independent.
    pub async fn run_pending(
        &self,
        engine: &Engine,
        session: SessionId,
    ) -> Result<DispatchReport, CoordinatorError> {
        let binding = self
            .inner
            .bindings
            .lock()
            .await
            .get(&session)
            .cloned()
            .ok_or(CoordinatorError::Closed)?;
        let mut stopped = binding.stop.subscribe();
        if *stopped.borrow() {
            return Err(CoordinatorError::Closed);
        }
        let guard = binding.supervisor.lock().await;
        if *stopped.borrow() {
            return Err(CoordinatorError::Closed);
        }
        let supervisor = guard.as_ref().ok_or(CoordinatorError::Closed)?;
        let profile = engine.profile().map_err(|_| CoordinatorError::Authority)?;
        let status = profile
            .status(session)
            .await
            .map_err(|_| CoordinatorError::Authority)?;
        if status.runnable {
            wake(engine, session);
        }
        let mut report = DispatchReport::default();
        let Some(turn) = status.open_turn else {
            return Ok(report);
        };
        for call in status.waiting.into_iter().filter(|call| owns(&call.name)) {
            let prior: Option<ClientDispatchRecord> = serde_json::from_value(
                profile
                    .command(
                        session,
                        SessionCommand::InspectClientDispatch {
                            turn,
                            call_id: call.id.clone(),
                        },
                    )
                    .await
                    .map_err(|_| CoordinatorError::Authority)?,
            )
            .map_err(|_| CoordinatorError::Authority)?;
            if let Some(record) = prior {
                if record.state == ClientDispatchState::Claimed {
                    if record.owner == self.inner.incarnation || record.deadline_ms <= status.now_ms
                    {
                        self.interrupt(engine, &record).await?;
                        report.interrupted += 1;
                    } else {
                        report.next_check_at_ms = Some(
                            report
                                .next_check_at_ms
                                .map_or(record.deadline_ms, |old| old.min(record.deadline_ms)),
                        );
                    }
                }
                continue;
            }
            if *stopped.borrow() {
                break;
            }
            let now_ms = profile
                .status(session)
                .await
                .map_err(|_| CoordinatorError::Authority)?
                .now_ms;
            let deadline_ms =
                now_ms.saturating_add(action_timeout(&call).as_millis() as i64 + 30_000);
            let claim: ClientDispatchClaimReply = serde_json::from_value(
                profile
                    .command(
                        session,
                        SessionCommand::BeginClientDispatch {
                            id: Uuid::new_v4().to_string(),
                            turn,
                            call: call.clone(),
                            owner: self.inner.incarnation,
                            resolution_id: Uuid::new_v4(),
                            deadline_ms,
                        },
                    )
                    .await
                    .map_err(|_| CoordinatorError::Authority)?,
            )
            .map_err(|_| CoordinatorError::Authority)?;
            if !claim.fresh {
                // Begin receipts are immutable and can describe an old claim.
                // Re-read the live record before deciding recovery; never run it.
                let current: Option<ClientDispatchRecord> = serde_json::from_value(
                    profile
                        .command(
                            session,
                            SessionCommand::InspectClientDispatch {
                                turn,
                                call_id: call.id.clone(),
                            },
                        )
                        .await
                        .map_err(|_| CoordinatorError::Authority)?,
                )
                .map_err(|_| CoordinatorError::Authority)?;
                if let Some(record) = current
                    && record.state == ClientDispatchState::Claimed
                {
                    let now = profile
                        .status(session)
                        .await
                        .map_err(|_| CoordinatorError::Authority)?
                        .now_ms;
                    if record.owner == self.inner.incarnation || record.deadline_ms <= now {
                        self.interrupt(engine, &record).await?;
                        report.interrupted += 1;
                    } else {
                        report.next_check_at_ms = Some(
                            report
                                .next_check_at_ms
                                .map_or(record.deadline_ms, |old| old.min(record.deadline_ms)),
                        );
                    }
                }
                continue;
            }
            let outcome = tokio::select! {
                biased;
                _=stopped.changed()=>None,
                result=tokio::time::timeout(action_timeout(&call),execute(supervisor,session,&call))=>Some(result.unwrap_or(Err(BrowseError::Interrupted))),
            };
            let Some(outcome) = outcome else {
                self.interrupt(engine, &claim.record).await?;
                report.interrupted += 1;
                break;
            };
            if matches!(
                outcome,
                Err(BrowseError::Interrupted | BrowseError::ProtocolError)
            ) {
                self.interrupt(engine, &claim.record).await?;
                report.interrupted += 1;
                continue;
            }
            let result = model::completed(&call, outcome);
            let completion = profile
                .command(
                    session,
                    SessionCommand::CompleteClientDispatch {
                        id: claim.record.resolution_id.to_string(),
                        turn,
                        call_id: call.id,
                        owner: self.inner.incarnation,
                        fence: claim.record.fence,
                        result: Box::new(result),
                    },
                )
                .await;
            // The transaction can commit even if its reply is lost. A wake is
            // safe in both cases: activation rechecks the authoritative state.
            wake(engine, session);
            completion.map_err(|_| CoordinatorError::Authority)?;
            report.completed += 1;
        }
        Ok(report)
    }
    async fn interrupt(
        &self,
        engine: &Engine,
        record: &ClientDispatchRecord,
    ) -> Result<(), CoordinatorError> {
        let result = engine
            .profile()
            .map_err(|_| CoordinatorError::Authority)?
            .command(
                record.session,
                SessionCommand::InterruptClientDispatch {
                    id: record.resolution_id.to_string(),
                    turn: record.turn,
                    call_id: record.call.id.clone(),
                    owner: self.inner.incarnation,
                    fence: record.fence,
                },
            )
            .await;
        wake(engine, record.session);
        result.map_err(|_| CoordinatorError::Authority)?;
        Ok(())
    }
    /// Independent cancellation signal interrupts a pending handoff before
    /// waiting for the per-session gate. Closed bindings cannot silently restart.
    pub async fn close_session(
        &self,
        engine: &Engine,
        session: SessionId,
    ) -> Result<(), CoordinatorError> {
        let binding = self.inner.bindings.lock().await.get(&session).cloned();
        if let Some(binding) = binding {
            binding.stop.send_replace(true);
            // Remove future exposure before resolving waits can resume a model.
            let first_cleanup = configure(engine, session, false).await;
            let mut guard = binding.supervisor.lock().await;
            // An enable already in flight may have committed after the first
            // removal. Finalize under the same session gate before cleanup.
            let final_cleanup = configure(engine, session, false).await;
            if let Some(supervisor) = guard.take() {
                supervisor.close().await;
            }
            first_cleanup?;
            final_cleanup?;
            let profile = engine.profile().map_err(|_| CoordinatorError::Authority)?;
            let status = profile
                .status(session)
                .await
                .map_err(|_| CoordinatorError::Authority)?;
            if let Some(turn) = status.open_turn {
                for call in status.waiting.into_iter().filter(|call| owns(&call.name)) {
                    let prior: Option<ClientDispatchRecord> = serde_json::from_value(
                        profile
                            .command(
                                session,
                                SessionCommand::InspectClientDispatch {
                                    turn,
                                    call_id: call.id.clone(),
                                },
                            )
                            .await
                            .map_err(|_| CoordinatorError::Authority)?,
                    )
                    .map_err(|_| CoordinatorError::Authority)?;
                    let record = if let Some(record) = prior {
                        record
                    } else {
                        let claim: ClientDispatchClaimReply = serde_json::from_value(
                            profile
                                .command(
                                    session,
                                    SessionCommand::BeginClientDispatch {
                                        id: Uuid::new_v4().to_string(),
                                        turn,
                                        call,
                                        owner: self.inner.incarnation,
                                        resolution_id: Uuid::new_v4(),
                                        deadline_ms: status.now_ms + 30_000,
                                    },
                                )
                                .await
                                .map_err(|_| CoordinatorError::Authority)?,
                        )
                        .map_err(|_| CoordinatorError::Authority)?;
                        claim.record
                    };
                    if record.state == ClientDispatchState::Claimed {
                        self.interrupt(engine, &record).await?;
                    }
                }
            }
            if profile.process_scheduler() {
                let engine = engine.clone();
                everruns_contracts::rt::spawn(async move {
                    let _ = engine.activate(session).await;
                });
            }
        }
        Self::disable_session(engine, session).await
    }
}
struct Declaration {
    name: &'static str,
    description: &'static str,
    parameters: Value,
}

fn wake(engine: &Engine, session: SessionId) {
    if engine
        .profile()
        .is_ok_and(|profile| profile.process_scheduler())
    {
        let engine = engine.clone();
        everruns_contracts::rt::spawn(async move {
            let _ = engine.activate(session).await;
        });
    }
}

#[cfg(test)]
mod tests;
