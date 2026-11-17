//! Agents using agents on the data plane (data plane v2, step 11.4): a
//! session's subagents run as sessions of their own, placed by the gateway
//! on whichever cell takes them, and its tasks live in its own cell state
//! (`tasks.rs`).
//!
//! Decisions:
//! - A child runs the parent's own agent definition, billed to the parent's
//!   org. The control plane's agent catalog is not on the data plane, so an
//!   agent id other than the parent's is refused rather than guessed.
//! - The store reaches only sessions this parent created: the children this
//!   activation made, and any child a task of the parent links to (which
//!   outlives the activation). Anything else reads as missing.
//! - The child is created and driven through the gateway's own calls, from
//!   a node (its gateway) or an executor (one it keeps), so a child on
//!   another cell is the same as one on this cell.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_capabilities::{
    PlatformCreateSessionRequest, PlatformHostBackendsExt, PlatformMessage, PlatformStore,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{
    AgentId, HarnessId, MessageId, SessionId, SessionParticipantId,
};
use everruns_core::events::Event;
use everruns_core::host::{HostBackends, SessionBuilder};
use everruns_core::session::ExecutionSession;
use everruns_core::{AgentDefinition, HarnessDefinition};

use crate::api::{Lookup, SessionView};
use crate::gateway::Gateway;
use crate::hosting::Port;
use crate::tasks::CellTasks;

/// How long a child's turn may run before `wait_for_idle` gives up on its
/// own (the tool passes its own limit too).
const LONGEST_WAIT: Duration = Duration::from_secs(30 * 60);

/// Give `session`'s activation its task registry and, when the gateway is
/// reachable, the platform store its subagents run through.
pub(crate) async fn attach(session: SessionId, backends: HostBackends, port: Port) -> HostBackends {
    let backends = backends.with_session_task_registry(Arc::new(CellTasks::new(port.clone())));
    let Some(gateway) = port.gateway() else {
        return backends;
    };
    let store: Arc<dyn PlatformStore> = Arc::new(CellPlatformStore {
        parent: session,
        port,
        gateway,
        children: Default::default(),
    });
    backends.with_platform_store_factory(Arc::new(move |_org, _session| store.clone()))
}

fn api(error: (axum::http::StatusCode, String)) -> AgentLoopError {
    AgentLoopError::store(format!("gateway: {} {}", error.0, error.1))
}

fn unsupported(op: &str) -> AgentLoopError {
    AgentLoopError::tool(format!(
        "'{op}' is not available to agents on the data plane"
    ))
}

/// The platform store of one parent session on a cell.
struct CellPlatformStore {
    parent: SessionId,
    port: Port,
    gateway: Arc<Gateway>,
    /// Children made by this activation.
    children: parking_lot::Mutex<HashMap<SessionId, ExecutionSession>>,
}

impl CellPlatformStore {
    /// Whether `child` is one of the parent's: made here, or linked from
    /// one of its tasks.
    async fn owns(&self, child: SessionId) -> Result<bool> {
        if self.children.lock().contains_key(&child) {
            return Ok(true);
        }
        let wanted = child.to_string();
        self.port
            .read(self.parent, move |s| {
                s.platform
                    .tasks
                    .values()
                    .any(|task| task["links"]["child_session_id"] == wanted.as_str())
            })
            .await
    }

    async fn own(&self, child: SessionId) -> Result<()> {
        if self.owns(child).await? {
            Ok(())
        } else {
            Err(AgentLoopError::session_not_found(child))
        }
    }

    async fn view(&self, session: SessionId) -> Result<SessionView> {
        let gateway = self.gateway.clone();
        self.gateway
            .with_cell(session, move |cell| {
                let gateway = gateway.clone();
                async move {
                    gateway
                        .call::<Lookup<SessionView>>(
                            cell,
                            &format!("/cell/sessions/{session}"),
                            None,
                        )
                        .await
                }
            })
            .await
            .map_err(api)
    }

    async fn events(&self, session: SessionId) -> Result<Vec<Event>> {
        let gateway = self.gateway.clone();
        self.gateway
            .with_cell(session, move |cell| {
                let gateway = gateway.clone();
                async move {
                    gateway
                        .call::<Lookup<Vec<Event>>>(
                            cell,
                            &format!("/cell/sessions/{session}/events?after=0"),
                            None,
                        )
                        .await
                }
            })
            .await
            .map_err(api)
    }
}

/// The text of the message an event carries, when it has any.
fn text_of(event: &Event) -> Option<String> {
    let data = serde_json::to_value(&event.data).ok()?;
    let parts = data["message"]["content"].as_array()?;
    let text: Vec<&str> = parts
        .iter()
        .filter_map(|p| p["text"].as_str())
        .filter(|t| !t.is_empty())
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

/// The outcome of the latest finished turn, as the platform store contract
/// reports it; `None` while one runs or before any finished.
fn outcome(events: &[Event]) -> Option<&'static str> {
    let mut status = None;
    for event in events {
        status = match event.event_type.as_str() {
            everruns_core::TURN_STARTED => None,
            everruns_core::TURN_COMPLETED => Some("completed"),
            everruns_core::TURN_FAILED => Some("failed"),
            everruns_core::TURN_CANCELLED => Some("cancelled"),
            everruns_core::TURN_SEALED => Some("sealed"),
            _ => continue,
        };
    }
    status
}

#[async_trait]
impl PlatformStore for CellPlatformStore {
    async fn get_harness(&self, _id: HarnessId) -> Result<Option<HarnessDefinition>> {
        Err(unsupported("get_harness"))
    }

    async fn get_agent_by_id(&self, _id: AgentId) -> Result<Option<AgentDefinition>> {
        Err(unsupported("get_agent_by_id"))
    }

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        if request.blueprint_id.is_some()
            || request.forked_from_session_id.is_some()
            || request.seed != everruns_core::session::SessionSeedMode::Fresh
        {
            return Err(unsupported("blueprints, forks and seeded subagents"));
        }
        let (definition, tenant, record) = self
            .port
            .read(self.parent, |s| {
                (
                    s.definition.clone(),
                    s.tenant.clone(),
                    s.authority.execution.clone(),
                )
            })
            .await?;
        let parent_agent = record
            .and_then(|r| serde_json::from_value::<ExecutionSession>(r).ok())
            .and_then(|r| r.agent_id);
        if request.agent_id.is_some() && request.agent_id != parent_agent {
            return Err(AgentLoopError::tool(
                "on the data plane a subagent runs its parent's own agent",
            ));
        }
        let id = SessionId::new();
        self.gateway
            .create((*definition).clone(), None, Some(id), tenant)
            .await
            .map_err(api)?;
        let mut child = SessionBuilder::new(request.harness_id)
            .id(id)
            .title(request.title.unwrap_or_else(|| "Subagent".into()))
            .build();
        child.agent_id = request.agent_id;
        child.goal = request.goal;
        child.locale = request.locale;
        child.parent_session_id = request.parent_session_id;
        self.children.lock().insert(id, child.clone());
        Ok(child)
    }

    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
        if let Some(child) = self.children.lock().get(&id) {
            return Ok(Some(child.clone()));
        }
        if !self.owns(id).await? {
            return Ok(None);
        }
        // A child from an earlier activation: what its task knows.
        let mut child = SessionBuilder::new(HarnessId::new()).id(id).build();
        child.parent_session_id = Some(self.parent);
        Ok(Some(child))
    }

    async fn add_agent_session_participant(
        &self,
        _session_id: SessionId,
        _agent_id: AgentId,
        _reason: &str,
    ) -> Result<SessionParticipantId> {
        Err(unsupported("add_agent_session_participant"))
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.own(session_id).await?;
        self.gateway
            .admit(session_id, content.to_owned(), Some(MessageId::new()))
            .await
            .map(|_| ())
            .map_err(api)
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        self.own(session_id).await?;
        let events = self.events(session_id).await?;
        let mut messages: Vec<PlatformMessage> = events
            .iter()
            .filter_map(|event| {
                let role = match event.event_type.as_str() {
                    "input.message" => "user",
                    "output.message.completed" => "agent",
                    _ => return None,
                };
                Some(PlatformMessage {
                    role: role.to_owned(),
                    content: text_of(event)?,
                    created_at: event.ts,
                })
            })
            .collect();
        // Most recent first, as the platform store contract reads them.
        messages.reverse();
        if let Some(limit) = limit {
            messages.truncate(limit);
        }
        Ok(messages)
    }

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        self.own(session_id).await?;
        let limit = timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(LONGEST_WAIT)
            .min(LONGEST_WAIT);
        let deadline = Instant::now() + limit;
        loop {
            let view = self.view(session_id).await?;
            // Idle: nothing holds it, nothing waits, no call is out.
            if view.active.is_none() && view.inbox == 0 && view.detached == 0 {
                let events = self.events(session_id).await?;
                // Admitted but not started yet reads as running.
                if let Some(outcome) = outcome(&events) {
                    return Ok(outcome.to_owned());
                }
                if view.turns_completed > 0 {
                    return Ok("idle".to_owned());
                }
            }
            if Instant::now() >= deadline {
                return Ok("timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

#[cfg(test)]
#[path = "subagents_tests.rs"]
mod tests;
