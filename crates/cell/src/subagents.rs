//! Agents using agents on the data plane (data plane v2, step 11.4): a
//! session's subagents run as sessions of their own, placed by the gateway
//! on whichever cell takes them, and its tasks live in its own cell state
//! (`tasks.rs`).
//!
//! Decisions:
//! - A child of the parent's own agent runs that agent's current revision
//!   (the key the control plane created the parent with,
//!   `Platform::agent_ref`), never the parent's own definition, which may
//!   carry the parent session's own settings (parity gap 4); it keeps the
//!   parent's pinned sandbox. A parent with no such key (a session made
//!   without a control plane) gives its children its own definition.
//! - Another agent is found in the org's catalog (parity gap 9): the
//!   control plane pushes one card per agent, naming it by the platform's
//!   ids, which coordination, handoff and teams carry in their
//!   configuration. The child runs that agent on its own harness. The
//!   catalog is the org's; a tool's own allow list (coordination's workers,
//!   handoff targets) still bounds which agents it starts.
//! - Reads of the parent's own agent and harness name them by the Engine's
//!   session-local ids, and answer from the parent's card.
//! - Billed to the parent's org.
//! - A detached child records its lineage only (`forked_from_session_id`
//!   with a fresh seed). Seeded children (a fork, a workspace copy) and
//!   blueprints are not on the data plane yet.
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

use everruns_contracts::runtime::saved_scripts::SavedScripts;

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
    let lookups = crate::lookups::org_lookups(port.material(), session);
    let calls = lookups
        .clone()
        .map(|lookups| crate::platform_calls::PlatformCalls::new(lookups, session));
    // The shell's saved scripts (`saved_scripts.rs`). A save is allowed here
    // and decided by the control plane, which reads the agent's shell
    // settings and the turn's person.
    let scripts = lookups.clone().map(|lookups| SavedScripts {
        store: Arc::new(crate::saved_scripts::CellSavedScripts::new(
            lookups, session,
        )),
        can_save: true,
    });
    let bind = calls.is_some();
    let store: Arc<dyn PlatformStore> = Arc::new(CellPlatformStore {
        parent: session,
        port,
        gateway,
        children: Default::default(),
        calls,
        lookups,
    });
    let mut backends = backends.with_platform_store_factory({
        let store = store.clone();
        Arc::new(move |_org, _session| store.clone())
    });
    // The platform tools' calls run as the person of each turn's input
    // message (`platform_calls.rs`): bind the store to it.
    if bind {
        let binding = Arc::new(crate::platform_calls::binding(store, scripts.clone()));
        let inner = backends.tool_context_extensions_factory.take();
        backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
            let mut extensions = inner
                .as_ref()
                .map(|factory| factory(org, session))
                .unwrap_or_default();
            extensions.insert(binding.clone());
            if let Some(scripts) = &scripts {
                extensions.insert(Arc::new(scripts.clone()));
            }
            extensions
        }));
    }
    backends
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
    /// Children made by this activation, shared with its bound copies.
    children: Arc<parking_lot::Mutex<HashMap<SessionId, ExecutionSession>>>,
    /// The platform tools' calls, when the executor reaches a control plane.
    calls: Option<crate::platform_calls::PlatformCalls>,
    /// The org's host calls, for a child's helper model.
    lookups: Option<crate::lookups::OrgLookups>,
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

/// What a parent's children and reads need from it.
struct Parent {
    definition: Arc<serde_json::Value>,
    /// Its org, when the control plane created it.
    org: Option<String>,
    agent_ref: Option<String>,
    /// The Engine's session-local ids of its agent and harness.
    agent: Option<AgentId>,
    harness: Option<HarnessId>,
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| AgentLoopError::store(error.to_string()))
}

impl CellPlatformStore {
    async fn parent(&self) -> Result<Parent> {
        let (definition, org, agent_ref, record) = self
            .port
            .read(self.parent, |s| {
                (
                    s.definition.clone(),
                    s.tenant.clone(),
                    s.platform.agent_ref.clone(),
                    s.authority.execution.clone(),
                )
            })
            .await?;
        let record = record.and_then(|r| serde_json::from_value::<ExecutionSession>(r).ok());
        Ok(Parent {
            definition,
            org,
            agent_ref,
            agent: record.as_ref().and_then(|r| r.agent_id),
            harness: record.map(|r| r.harness_id),
        })
    }

    /// The org's agent the platform names `id`, from its catalog.
    async fn catalog(
        &self,
        parent: &Parent,
        id: AgentId,
    ) -> Result<Option<crate::dp::AgentDefinition>> {
        match &parent.org {
            Some(org) => self
                .gateway
                .catalog_agent(org, &id.to_string())
                .await
                .map_err(api),
            None => Ok(None),
        }
    }

    /// The parent's own agent at its current revision, when the control
    /// plane created the parent.
    async fn parent_revision(&self, parent: &Parent) -> Result<Option<crate::dp::AgentDefinition>> {
        let (Some(org), Some(agent_ref)) = (&parent.org, &parent.agent_ref) else {
            return Ok(None);
        };
        self.gateway
            .agent_revision(org, agent_ref)
            .await
            .map(Some)
            .map_err(api)
    }

    async fn parent_card(&self, parent: &Parent) -> Result<Option<crate::dp::AgentCard>> {
        Ok(self
            .parent_revision(parent)
            .await?
            .and_then(|revision| revision.card))
    }

    /// The `model` a child's definition takes for its helper key: the
    /// parent's own for `same` (the session's model rides in its
    /// definition), else what the control plane resolves the key to
    /// (host call `helper_model`). `None` keeps the child's agent model,
    /// also on a cell without a control plane.
    async fn helper_model(
        &self,
        parent: &Parent,
        helper: &everruns_core::helper_models::SpawnHelperModel,
    ) -> Result<Option<serde_json::Value>> {
        use everruns_core::helper_models::HelperModelTarget;
        use everruns_internal_protocol::commands::InternalCommandTransport;
        if helper.target == HelperModelTarget::Same {
            return Ok(parent.definition.get("model").cloned());
        }
        let Some(lookups) = &self.lookups else {
            return Ok(None);
        };
        let answer = lookups
            .execute_internal_command(
                "worker_resolve_helper_model",
                serde_json::json!({ "session_id": self.parent.to_string(), "helper": helper }),
            )
            .await?;
        Ok(match answer {
            Ok(choice) if choice.is_object() => Some(serde_json::json!({
                "provider": choice["provider"],
                "model": choice["model"],
            })),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(error = %error.message, "helper model lookup refused; keeping the agent model");
                None
            }
        })
    }

    /// What a child of the parent's own agent runs, and its agent key.
    async fn parent_agent(&self, parent: &Parent) -> Result<(serde_json::Value, Option<String>)> {
        let Some(revision) = self.parent_revision(parent).await? else {
            return Ok(((*parent.definition).clone(), None));
        };
        let mut package = revision.package;
        let sandbox = parent.definition["capabilities"]
            .as_array()
            .and_then(|capabilities| {
                capabilities
                    .iter()
                    .find(|c| crate::sandbox::capability_entry(c).is_some())
            })
            .cloned();
        if let Some(entry) = sandbox {
            crate::gateway_dp::with_capability(&mut package, entry);
        }
        Ok((package, Some(revision.agent_id)))
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

impl CellPlatformStore {
    async fn platform(&self, operation: &str, arguments: serde_json::Value) -> Result<String> {
        match &self.calls {
            Some(calls) => calls.call(operation, arguments).await,
            None => Err(unsupported("platform commands without a control plane")),
        }
    }
}

#[async_trait]
impl PlatformStore for CellPlatformStore {
    fn for_execution(&self, input_message_id: uuid::Uuid) -> Option<Arc<dyn PlatformStore>> {
        let calls = self.calls.as_ref()?;
        Some(Arc::new(Self {
            parent: self.parent,
            port: self.port.clone(),
            gateway: self.gateway.clone(),
            children: self.children.clone(),
            calls: Some(calls.bound(MessageId::from_uuid(input_message_id))),
            lookups: self.lookups.clone(),
        }))
    }

    async fn platform_discover(&self, arguments: serde_json::Value) -> Result<String> {
        self.platform("discover", arguments).await
    }

    async fn platform_query(&self, arguments: serde_json::Value) -> Result<String> {
        self.platform("query", arguments).await
    }

    async fn platform_execute(&self, arguments: serde_json::Value) -> Result<String> {
        self.platform("execute", arguments).await
    }

    async fn platform_run(&self, command: &str, params: serde_json::Value) -> Result<String> {
        self.platform(
            "run",
            serde_json::json!({ "command": command, "params": params }),
        )
        .await
    }

    async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>> {
        let parent = self.parent().await?;
        let card = if parent.harness == Some(id) {
            self.parent_card(&parent).await?.map(|card| card.harness)
        } else {
            match &parent.org {
                Some(org) => self
                    .gateway
                    .catalog_harness(org, &id.to_string())
                    .await
                    .map_err(api)?,
                None => None,
            }
        };
        card.map(decode).transpose()
    }

    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<AgentDefinition>> {
        let parent = self.parent().await?;
        let card = if parent.agent == Some(id) {
            self.parent_card(&parent).await?
        } else {
            self.catalog(&parent, id)
                .await?
                .and_then(|found| found.card)
        };
        card.map(|card| decode(card.agent)).transpose()
    }

    async fn get_agent_harness_id(&self, id: AgentId) -> Result<Option<HarnessId>> {
        let parent = self.parent().await?;
        if parent.agent == Some(id) {
            return Ok(parent.harness);
        }
        let Some(card) = self
            .catalog(&parent, id)
            .await?
            .and_then(|found| found.card)
        else {
            return Ok(None);
        };
        card.harness_id
            .parse()
            .map(Some)
            .map_err(|_| AgentLoopError::store(format!("catalog harness id {}", card.harness_id)))
    }

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        if request.blueprint_id.is_some() {
            return Err(unsupported("blueprint subagents"));
        }
        if request.seed != everruns_core::session::SessionSeedMode::Fresh {
            return Err(unsupported("forked and workspace-seeded subagents"));
        }
        let parent = self.parent().await?;
        let (definition, agent_ref) = match request.agent_id {
            Some(id) if parent.agent != Some(id) => {
                let found = self
                    .catalog(&parent, id)
                    .await?
                    .ok_or_else(|| AgentLoopError::tool(format!("Agent {id} was not found.")))?;
                let own = found
                    .card
                    .as_ref()
                    .is_some_and(|card| card.harness_id == request.harness_id.to_string());
                if !own && parent.harness != Some(request.harness_id) {
                    return Err(AgentLoopError::tool(format!(
                        "on the data plane agent {id} runs on its own harness"
                    )));
                }
                (found.package, Some(found.agent_id))
            }
            _ => self.parent_agent(&parent).await?,
        };
        let mut definition = definition;
        if let Some(helper) = &request.helper_model
            && let Some(model) = self.helper_model(&parent, helper).await?
        {
            definition["model"] = model;
        }
        // Decision: a child gets a session record on the control plane
        // first (`subagent_record`), as a v1 subagent has, and runs under the
        // id that record has, with the Memory mounts resolved from it. A cell
        // with no control plane (standalone) starts it without one.
        let (id, memory) = match (&self.calls, &parent.org) {
            (Some(calls), Some(_)) => {
                calls
                    .record_subagent(serde_json::json!({
                        "parent_session_id": self.parent.to_string(),
                        "harness_id": request.harness_id.to_string(),
                        "agent_id": request.agent_id.map(|id| id.to_string()),
                        "title": request.title.clone(),
                        "locale": request.locale.clone(),
                    }))
                    .await?
            }
            _ => (SessionId::new(), Vec::new()),
        };
        self.gateway
            .create_running(
                definition,
                None,
                Some(id),
                parent.org.clone(),
                agent_ref,
                memory,
            )
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
        child.forked_from_session_id = request.forked_from_session_id;
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
