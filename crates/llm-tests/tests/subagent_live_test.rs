// Live end-to-end test for background spawn_agent subagent delegation against a real LLM
// through the runtime: InProcessRuntime + the local-host PlatformStore
// (`everruns-local` owns the embeddable PlatformStore implementation the
// subagent tools require; `everruns-host` deliberately has none) + the
// SQLite LocalSessionTaskRegistry.
//
// No mocks anywhere in the loop:
// - a real model decides to call spawn_agent from plain instructions
// - the tool returns while the parent turn is still running (background default)
// - the detached watcher creates the child session and drives its REAL LLM turn
// - the task settles Succeeded with the child's actual reply as the summary
//
// Skips gracefully when ANTHROPIC_API_KEY is absent (matrix convention). Run:
//   doppler run -- cargo test -p everruns-llm-tests --test subagent_live_test
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use everruns_host::HostComposition;
use llm_test_matrix::*;

use async_trait::async_trait;
use everruns_core::session::ExecutionSession;
use everruns_core::session_task::{SessionTaskRegistry, SessionTaskState};
use everruns_core::{CapabilityRegistry, MessageRole};
use everruns_host::{
    AgentBuilder, HarnessBuilder, HostBackends, InProcessRuntime, InProcessRuntimeBuilder,
    RuntimeSessionStore, SessionBuilder,
};
use everruns_local::{LocalPlatformStore, LocalSessionRunner, LocalSessionTaskRegistry, SqliteDb};
use everruns_platform::capabilities::SubagentCapability;
use everruns_platform::{PlatformMessage, PlatformStore};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const CHILD_MARKER: &str = "EVERRUNS_LIVE_OK";

/// LocalSessionRunner over the runtime it is embedded in — the wiring every
/// embedder does per the seam docs on `LocalSessionRunner`. The runtime handle
/// arrives late (chicken/egg: the store factory is set before build) via a
/// OnceLock; the session store handle is shared with the runtime's backends.
struct RuntimeRunner {
    runtime: Arc<OnceLock<InProcessRuntime>>,
    sessions: Arc<dyn RuntimeSessionStore>,
}

impl RuntimeRunner {
    fn runtime(&self) -> Result<&InProcessRuntime> {
        self.runtime.get().ok_or_else(|| {
            everruns_provider::error::AgentLoopError::config("runtime not initialized yet")
        })
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
            .title(title.unwrap_or("subagent"))
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
            Err(everruns_provider::error::AgentLoopError::tool(format!(
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
        // Turns run synchronously inside send_message, so a polling caller
        // always observes the session idle — the bare-idle settle path.
        Ok(Some("idle".to_string()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn background_spawn_agent_subagent_live_end_to_end() {
    let config = ANTHROPIC_HAIKU;
    let Some(model_config) = config.model() else {
        eprintln!("Skipping: {} not set", config.label());
        return;
    };

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(SubagentCapability);
    let platform = HostComposition::new(capabilities, all_providers_registry());

    let harness_id = HarnessId::from_seed(535);
    let agent_id = AgentId::from_seed(535);
    let parent_id = SessionId::from_seed(535);
    let harness = HarnessBuilder::new("host", "You are a precise assistant that uses tools.")
        .id(harness_id)
        .capability("subagents")
        .build();
    let agent = AgentBuilder::new("host-agent", "Use tools exactly as instructed.")
        .id(agent_id)
        .max_iterations(6)
        .build();
    let parent = SessionBuilder::new(harness_id)
        .id(parent_id)
        .agent(agent_id)
        .title("live parent")
        .build();

    let (model, provider_config) = model_config.into_parts();
    let provider_store = Arc::new(everruns_host::InMemoryProviderStore::new());
    if let Some(provider_config) = provider_config {
        provider_store.set_provider_config(provider_config).await;
    }
    let backends = HostBackends::in_memory().with_provider_store(provider_store);
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
        .default_model(model)
        .harness(harness)
        .agent(agent)
        .session(parent)
        .build()
        .await
        .expect("runtime builds");
    runtime_cell.set(runtime.clone()).ok().expect("set once");

    // 1. Parent turn: the real model must choose to call spawn_agent.
    let turn = runtime
        .run_text_turn(
            parent_id,
            &format!(
                "Call the spawn_agent tool once, with target.type \"subagent\", name \"Echo\", \
                 and instructions \"Reply with exactly the word {CHILD_MARKER} and nothing else.\". \
                 Do not pass a mode. After the tool returns, tell me the task id it reported."
            ),
        )
        .await
        .expect("parent turn runs");
    skip_if_quota!(turn, config.label());
    assert!(turn.success, "parent turn failed: {:?}", turn.error);

    let parent_messages = runtime.messages(parent_id).await.expect("parent messages");
    let spawn_call = parent_messages
        .iter()
        .flat_map(|m| m.tool_calls())
        .find(|tc| tc.name == "spawn_agent")
        .expect("the live model should have called spawn_agent");
    assert!(
        spawn_call.arguments.to_string().contains(CHILD_MARKER),
        "spawn args should carry the child instructions: {}",
        spawn_call.arguments
    );

    // 2. Background contract: the tool returned a running task during the turn.
    let tasks = registry.list(parent_id, None).await.expect("list tasks");
    let task = tasks
        .iter()
        .find(|t| t.kind == "subagent")
        .expect("spawn_agent should have registered a subagent task");
    assert_eq!(task.spec["mode"], "background");

    // 3. The detached watcher drives the child's real turn; wait for settle.
    let mut settled = None;
    for _ in 0..120 {
        let current = registry
            .get(parent_id, &task.id)
            .await
            .expect("get task")
            .expect("task exists");
        if current.state.is_terminal() {
            settled = Some(current);
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let settled = settled.expect("subagent task should settle within 120s");
    // The child's turn is a second live call; out-of-quota there is the same
    // provider condition as on the parent turn, not a subagent regression.
    if settled.state != SessionTaskState::Succeeded
        && let Some(err) = settled.error.as_ref()
        && is_quota_exhausted(&err.message)
    {
        eprintln!(
            "SKIP: provider {} out of quota: {}",
            config.label(),
            err.message
        );
        return;
    }
    assert_eq!(
        settled.state,
        SessionTaskState::Succeeded,
        "task error: {:?}",
        settled.error
    );

    // 4. The summary is the child's REAL model reply.
    let summary = settled.summary.as_deref().expect("settled task summary");
    assert!(
        summary.contains(CHILD_MARKER),
        "child's live reply should contain {CHILD_MARKER}: {summary}"
    );

    // 5. The child is a real session linked to the parent, with a real transcript.
    let child_id: SessionId = settled.links.child_session_id.expect("child session link");
    let child_messages = runtime.messages(child_id).await.expect("child messages");
    let child_reply = child_messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Agent)
        .and_then(|m| m.text())
        .expect("child agent reply");
    assert!(
        child_reply.contains(CHILD_MARKER),
        "child said: {child_reply}"
    );

    println!("parent turn iterations: {}", turn.iterations);
    println!("task settled: {} — summary: {summary}", settled.state);
    println!("child reply: {child_reply}");
}
