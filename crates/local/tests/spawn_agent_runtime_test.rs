// EVE-677 runtime proof: the unified `spawn_agent` dispatcher driven end-to-end
// through the in-process runtime with the deterministic llmsim driver (no API
// key, no mocks in the delegation path).
//
// This is the "Runtime proof" the EVE-677 spec asked for — spawning a subagent
// AND a handoff via `InProcessRuntimeBuilder`, asserting the returned task and
// the child result. It lives in `everruns-local`, not `everruns-host`,
// because the embeddable `PlatformStore` the delegation tools require lives here
// (`everruns-host` deliberately has none — see `LocalPlatformStore`). Adding
// `everruns-local` as a dev-dependency of `everruns-host` would form a
// publish-hostile crate cycle, so the proof sits on this side of the seam, the
// same reason the live sibling test lives in `everruns-llm-tests`.
//
// What it exercises against a real assembled runtime:
// - The single `spawn_agent` tool composes the union `target.type` enum from the
//   active delegation capabilities (subagents + agent_handoff) — EVE-677 AC #6.
// - `target.type = subagent`, background mode → `subagent` task, settled via the
//   detached watcher's bare-`idle`→`completed` local-host path, summary carrying
//   the child's real reply — EVE-677 AC #1.
// - `target.type = agent`, foreground mode → `agent_handoff` task (its own kind,
//   not `subagent`) settled inline with the target agent's reply — AC #1/#3.
//
// Determinism note: one llmsim driver serves the parent and every child. Tool
// calls are content-keyed (`ToolCallConfig::Conditional`) so parent/child
// scheduling order is irrelevant. The subagent and handoff proofs use separate
// parent sessions inside the same runtime so a background completion wake or
// previous trigger in one transcript cannot steer the other target's turn.

use everruns_host::HostComposition;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::error::Result;
use everruns_core::session::ExecutionSession;
use everruns_core::session_task::{
    SessionTaskRegistry, SessionTaskState, TASK_KIND_AGENT_HANDOFF, TASK_KIND_SUBAGENT,
};
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{
    AgentCapabilityConfig, CapabilityRegistry, DriverId, MessageRole, ResolvedModel, ToolCall,
};
use everruns_host::{
    AgentBuilder, HarnessBuilder, HostBackends, InProcessRuntime, InProcessRuntimeBuilder,
    RuntimeSessionStore, SessionBuilder,
};
use everruns_local::{LocalPlatformStore, LocalSessionRunner, LocalSessionTaskRegistry, SqliteDb};
use everruns_platform::capabilities::{AgentHandoffCapability, SubagentCapability};
use everruns_platform::{PlatformMessage, PlatformStore};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::llmsim_driver::{
    LlmSimConfig, ResponseConfig, ToolCallConfig, ToolCallPattern,
};

/// Substrings that steer the content-keyed llmsim: a parent prompt containing
/// one of these makes the model emit the matching `spawn_agent` call. Child
/// instructions contain neither, so a child turn never re-delegates.
const TRIGGER_SUBAGENT: &str = "DELEGATE_TO_SUBAGENT";
const TRIGGER_HANDOFF: &str = "DELEGATE_TO_HANDOFF";
/// Markers embedded in each child's instructions; the child echoes them back so
/// the settled task summary proves the *specific* child actually ran.
const SUBAGENT_MARKER: &str = "SUBAGENT_CHILD_MARKER";
const HANDOFF_MARKER: &str = "HANDOFF_CHILD_MARKER";

/// A `LocalSessionRunner` backed by the very `InProcessRuntime` it is embedded
/// in — the seam every embedder wires per the docs on `LocalSessionRunner`. The
/// runtime handle arrives after build (the store factory is set before build),
/// so it is threaded in late via a `OnceLock`; the session store is shared with
/// the runtime's backends so child sessions created here are runnable there.
struct RuntimeRunner {
    runtime: Arc<OnceLock<InProcessRuntime>>,
    sessions: Arc<dyn RuntimeSessionStore>,
}

impl RuntimeRunner {
    fn runtime(&self) -> Result<&InProcessRuntime> {
        self.runtime
            .get()
            .ok_or_else(|| everruns_core::AgentLoopError::config("runtime not initialized yet"))
    }
}

#[async_trait]
impl LocalSessionRunner for RuntimeRunner {
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        _locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        let mut session = SessionBuilder::new(harness_id)
            .id(SessionId::new())
            .title(title.unwrap_or("child"))
            .build();
        session.agent_id = agent_id;
        session.parent_session_id = parent_session_id;
        self.sessions.add_session(session.clone()).await?;
        Ok(session)
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let result = self.runtime()?.run_text_turn(session_id, content).await?;
        if result.success {
            Ok(())
        } else {
            Err(everruns_core::AgentLoopError::tool(format!(
                "child turn failed: {}",
                result.error.unwrap_or_default()
            )))
        }
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>> {
        Ok(vec![])
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.sessions.get_session(session_id).await
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        let messages = self.runtime()?.messages(session_id).await?;
        let mut mapped: Vec<PlatformMessage> = messages
            .iter()
            .map(|m| PlatformMessage {
                role: match &m.role {
                    MessageRole::Agent => "agent".to_string(),
                    MessageRole::User => "user".to_string(),
                    other => format!("{other:?}").to_lowercase(),
                },
                content: m.text().unwrap_or_default().to_string(),
                created_at: m.created_at,
            })
            .collect();
        if let Some(limit) = limit {
            let skip = mapped.len().saturating_sub(limit);
            mapped.drain(..skip);
        }
        Ok(mapped)
    }

    async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
        // Turns run synchronously to completion inside `send_message`, so by the
        // time anything polls, the child is idle. The background subagent watcher
        // maps bare `idle` → `completed`; a foreground handoff treats `idle` as
        // terminal directly. Either way the task settles.
        Ok(Some("idle".to_string()))
    }
}

/// One shared llmsim config for the whole runtime. `Echo` makes each child reply
/// with `"Echo: <its instructions>"` so the settled summary proves the specific
/// child ran; `Conditional` tool calls are content-keyed so parent turns emit
/// the right `spawn_agent` call regardless of parent/child scheduling order.
fn spawn_agent_sim(handoff_target_id: &str) -> LlmSimConfig {
    let subagent_call = ToolCall {
        id: "call_spawn_subagent".into(),
        name: "spawn_agent".into(),
        arguments: serde_json::json!({
            "name": "Subagent Echo",
            "instructions": format!("Acknowledge {SUBAGENT_MARKER} and reply briefly."),
            "target": { "type": "subagent" },
            "mode": "background",
        }),
    };
    let handoff_call = ToolCall {
        id: "call_spawn_handoff".into(),
        name: "spawn_agent".into(),
        arguments: serde_json::json!({
            "name": "Handoff Run",
            "instructions": format!("Acknowledge {HANDOFF_MARKER} and reply briefly."),
            "target": { "type": "agent", "id": handoff_target_id },
            "mode": "foreground",
        }),
    };

    LlmSimConfig {
        response: ResponseConfig::Echo,
        tool_calls: Some(ToolCallConfig::Conditional {
            patterns: vec![
                ToolCallPattern::new(TRIGGER_SUBAGENT, vec![subagent_call]),
                ToolCallPattern::new(TRIGGER_HANDOFF, vec![handoff_call]),
            ],
        }),
        simulate_latency: false,
        model_name: "llmsim-model".to_string(),
        response_delay: None,
        response_id: None,
        effort_capture: None,
        message_capture: None,
    }
}

fn llmsim_model() -> ResolvedModel {
    ResolvedModel {
        model: "llmsim-model".into(),
        provider_type: DriverId::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
        provider_metadata: None,
    }
}

/// Poll the registry until the task reaches a terminal state or the deadline
/// passes. The background subagent watcher settles asynchronously; with the
/// instant llmsim this is milliseconds, but we allow generous slack.
async fn await_terminal(
    registry: &dyn SessionTaskRegistry,
    session_id: SessionId,
    task_id: &str,
) -> everruns_core::session_task::SessionTask {
    for _ in 0..200 {
        let task = registry
            .get(session_id, task_id)
            .await
            .expect("get task")
            .expect("task exists");
        if task.state.is_terminal() {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {task_id} did not settle within the deadline");
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_dispatches_subagent_and_handoff_via_llmsim() {
    // Capabilities: subagents + agent_handoff are both registered and both
    // active on the parent, so the assembled `spawn_agent` advertises both
    // targets and dispatches to each.
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(SubagentCapability);
    capabilities.register(AgentHandoffCapability);
    let platform = HostComposition::new(capabilities, DriverRegistry::new());

    // Parent (root) session: harness carries subagents + a configured
    // agent_handoff target pointing at the plain target agent below.
    let parent_harness_id = HarnessId::from_seed(677);
    let parent_agent_id = AgentId::from_seed(677);
    let parent_subagent_session_id = SessionId::from_seed(677);
    let parent_handoff_session_id = SessionId::from_seed(679);
    // Handoff target: a plain agent with no delegation capabilities — it just
    // replies. Its harness/agent must be seeded so the child session is runnable.
    let target_harness_id = HarnessId::from_seed(678);
    let target_agent_id = AgentId::from_seed(678);
    const HANDOFF_TARGET_ID: &str = "target_one";

    let parent_harness = HarnessBuilder::new("orchestrator", "You delegate work to other agents.")
        .id(parent_harness_id)
        .capability("subagents")
        .capability(AgentCapabilityConfig::with_config(
            "agent_handoff",
            serde_json::json!({
                "targets": [{
                    "id": HANDOFF_TARGET_ID,
                    "name": "Target One",
                    "agent_id": target_agent_id,
                    "harness_id": target_harness_id,
                }]
            }),
        ))
        .build();
    // max_iterations(2): the first reason's spawn_agent call executes (Act); the
    // re-triggered second is generated but capped before Act. Exactly one spawn.
    let parent_agent = AgentBuilder::new("orchestrator-agent", "Use tools exactly as instructed.")
        .id(parent_agent_id)
        .max_iterations(2)
        .build();
    let parent_subagent_session = SessionBuilder::new(parent_harness_id)
        .id(parent_subagent_session_id)
        .agent(parent_agent_id)
        .title("EVE-677 parent subagent")
        .build();
    let parent_handoff_session = SessionBuilder::new(parent_harness_id)
        .id(parent_handoff_session_id)
        .agent(parent_agent_id)
        .title("EVE-677 parent handoff")
        .build();

    let target_harness = HarnessBuilder::new("target", "You are a specialist agent that replies.")
        .id(target_harness_id)
        .build();
    let target_agent = AgentBuilder::new("target-agent", "Acknowledge the task briefly.")
        .id(target_agent_id)
        .max_iterations(4)
        .build();

    let backends = HostBackends::in_memory();
    let sessions = backends.session_store.clone();
    let registry: Arc<dyn SessionTaskRegistry> = Arc::new(
        LocalSessionTaskRegistry::new(SqliteDb::open_in_memory().expect("sqlite"))
            .expect("task registry"),
    );

    let runtime_cell: Arc<OnceLock<InProcessRuntime>> = Arc::new(OnceLock::new());
    let store: Arc<dyn PlatformStore> = Arc::new(LocalPlatformStore::new(
        Arc::new(RuntimeRunner {
            runtime: runtime_cell.clone(),
            sessions,
        }),
        "http://localhost",
    ));

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .backends(backends)
        .with_session_task_registry(registry.clone())
        .with_platform_store_factory(Arc::new(move |_org, _session| store.clone()))
        .default_model(llmsim_model())
        .llm_sim(spawn_agent_sim(HANDOFF_TARGET_ID))
        .harness(parent_harness)
        .agent(parent_agent)
        .session(parent_subagent_session)
        .session(parent_handoff_session)
        .harness(target_harness)
        .agent(target_agent)
        .build()
        .await
        .expect("runtime builds");
    runtime_cell.set(runtime.clone()).ok().expect("set once");

    // AC #6: the single assembled `spawn_agent` advertises the union of the two
    // active delegation targets, in the dispatcher's canonical order.
    let context = runtime
        .load_context(parent_subagent_session_id)
        .await
        .expect("load context");
    let spawn_agent = context
        .runtime_agent
        .tools
        .iter()
        .find(|tool| tool.name() == "spawn_agent")
        .expect("assembled context exposes the unified spawn_agent tool");
    assert_eq!(
        spawn_agent.parameters()["properties"]["target"]["properties"]["type"]["enum"],
        serde_json::json!(["subagent", "agent"]),
        "spawn_agent must advertise exactly the active delegation targets",
    );

    // ---- Target 1: subagent (background) -----------------------------------
    let turn = runtime
        .run_text_turn(
            parent_subagent_session_id,
            format!("Please {TRIGGER_SUBAGENT} right now."),
        )
        .await
        .expect("parent subagent turn runs");
    assert!(
        turn.success,
        "parent subagent turn failed: {:?}",
        turn.error
    );

    let subagent_tasks = registry
        .list(parent_subagent_session_id, None)
        .await
        .expect("list tasks");
    assert_eq!(
        subagent_tasks
            .iter()
            .filter(|t| t.kind == TASK_KIND_SUBAGENT)
            .count(),
        1,
        "spawn_agent(subagent) should register exactly one subagent task"
    );
    let subagent_task = subagent_tasks
        .into_iter()
        .find(|t| t.kind == TASK_KIND_SUBAGENT)
        .expect("spawn_agent(subagent) registered a subagent task");
    assert_eq!(subagent_task.spec["mode"], "background");

    let settled = await_terminal(
        registry.as_ref(),
        parent_subagent_session_id,
        &subagent_task.id,
    )
    .await;
    assert_eq!(
        settled.state,
        SessionTaskState::Succeeded,
        "subagent task error: {:?}",
        settled.error
    );
    let summary = settled.summary.as_deref().expect("subagent task summary");
    assert!(
        summary.contains(SUBAGENT_MARKER),
        "subagent summary should carry the child's reply: {summary}"
    );
    // The child is a real, parent-linked session with a transcript.
    let child_id = settled
        .links
        .child_session_id
        .expect("subagent task links a child session");
    let child_reply = runtime
        .messages(child_id)
        .await
        .expect("child messages")
        .into_iter()
        .rev()
        .find(|m| m.role == MessageRole::Agent)
        .and_then(|m| m.text().map(str::to_string))
        .expect("child produced an agent reply");
    assert!(
        child_reply.contains(SUBAGENT_MARKER),
        "child transcript should contain its marker: {child_reply}"
    );

    // ---- Target 2: agent handoff (foreground) ------------------------------
    let turn = runtime
        .run_text_turn(
            parent_handoff_session_id,
            format!("Please {TRIGGER_HANDOFF} right now."),
        )
        .await
        .expect("parent handoff turn runs");
    assert!(turn.success, "parent handoff turn failed: {:?}", turn.error);

    // AC #3: the handoff has its OWN kind — `list_tasks(kind=subagent)` must not
    // return it in the handoff parent session.
    let tasks = registry
        .list(parent_handoff_session_id, None)
        .await
        .expect("list tasks");
    assert_eq!(
        tasks
            .iter()
            .filter(|t| t.kind == TASK_KIND_SUBAGENT)
            .count(),
        0,
        "the handoff must not be recorded under the subagent kind"
    );
    // Foreground settles inline, so the handoff task is already terminal.
    let handoff_task = tasks
        .iter()
        .find(|t| t.kind == TASK_KIND_AGENT_HANDOFF)
        .expect("spawn_agent(agent) registered an agent_handoff task");
    assert_eq!(handoff_task.spec["mode"], "foreground");
    assert_eq!(handoff_task.spec["target_id"], HANDOFF_TARGET_ID);
    assert_eq!(
        handoff_task.state,
        SessionTaskState::Succeeded,
        "handoff task error: {:?}",
        handoff_task.error
    );
    let handoff_summary = handoff_task.summary.as_deref().expect("handoff summary");
    assert!(
        handoff_summary.contains(HANDOFF_MARKER),
        "handoff summary should carry the target agent's reply: {handoff_summary}"
    );
}
