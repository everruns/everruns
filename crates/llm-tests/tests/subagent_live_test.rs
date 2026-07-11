// Live end-to-end test for background spawn_agent subagent delegation against a real LLM
// through the runtime: InProcessRuntime + the local-host PlatformStore
// (`everruns-local` owns the embeddable PlatformStore implementation the
// subagent tools require; `everruns-runtime` deliberately has none) + the
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

use llm_test_matrix::*;

use async_trait::async_trait;
use everruns_core::capabilities::SubagentCapability;
use everruns_core::error::Result;
use everruns_core::platform_store::{PlatformMessage, PlatformStore};
use everruns_core::session::Session;
use everruns_core::session_task::{SessionTaskRegistry, SessionTaskState};
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{CapabilityRegistry, MessageRole, PlatformDefinition};
use everruns_local::{LocalPlatformStore, LocalSessionRunner, LocalSessionTaskRegistry, SqliteDb};
use everruns_runtime::{
    AgentBuilder, HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder, RuntimeBackends,
    RuntimeSessionStore, SessionBuilder,
};
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
    ) -> Result<Session> {
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
    ) -> Result<Vec<Session>> {
        Ok(vec![])
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
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

/// Run one full live attempt of the subagent e2e flow.
///
/// Returns `Err(msg)` only when a *transient transport* blip (network/stream
/// decode hiccup) was observed at the parent turn or the child-turn settle — the
/// caller retries on that. Every real (assertion / functional) failure panics
/// inside this function so it fails loudly and immediately, keeping full
/// coverage rather than skipping (the deliberate policy behind reverting the
/// earlier skip-on-transient mechanism in #2550).
async fn run_subagent_live_attempt(
    config: &ProviderModelConfig,
) -> std::result::Result<(), String> {
    let model = config.model().expect("model set (checked by caller)");

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(SubagentCapability);
    let platform = PlatformDefinition::new(capabilities, all_providers_registry());

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

    let backends = RuntimeBackends::in_memory();
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
        .platform_definition(platform)
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

    // 1. Parent turn: the real model must choose to call spawn_agent. A transient
    //    transport/stream-decode blip here is retried by the caller, not failed.
    let turn = match runtime
        .run_text_turn(
            parent_id,
            &format!(
                "Call the spawn_agent tool once, with target.type \"subagent\", name \"Echo\", \
                 and instructions \"Reply with exactly the word {CHILD_MARKER} and nothing else.\". \
                 Do not pass a mode. After the tool returns, tell me the task id it reported."
            ),
        )
        .await
    {
        Ok(turn) => turn,
        Err(err) => {
            let msg = err.to_string();
            if is_transient_transport_error(&msg) {
                return Err(format!("parent turn transport error: {msg}"));
            }
            panic!("parent turn runs: {msg}");
        }
    };
    if !turn.success {
        let err = turn.error.clone().unwrap_or_default();
        if is_transient_transport_error(&err) {
            return Err(format!("parent turn failed transiently: {err}"));
        }
        panic!("parent turn failed: {:?}", turn.error);
    }

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
    if settled.state != SessionTaskState::Succeeded {
        // The child turn runs a real LLM through the watcher; a transient
        // transport blip there settles the task Failed with the transport error
        // wrapped in the child-turn message — retry rather than red CI.
        let err_msg = settled
            .error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_default();
        if is_transient_transport_error(&err_msg) {
            return Err(format!(
                "child task settled with transport error: {err_msg}"
            ));
        }
        panic!(
            "subagent task did not succeed: state={:?} error={:?}",
            settled.state, settled.error
        );
    }

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
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn background_spawn_agent_subagent_live_end_to_end() {
    let config = ANTHROPIC_HAIKU;
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    // Live e2e against a real provider: a transient transport/stream-decode blip
    // (e.g. "Transport error: error decoding response body") is infrastructure
    // flakiness, not a regression, and must not red main CI. Retry the whole flow
    // on transient transport faults (fresh runtime state each attempt) while
    // still failing loudly and immediately on any real assertion/functional
    // failure. Mirrors the bounded retry the rest of the live matrix uses via
    // `run_live_turn!` (EVE-736).
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        match run_subagent_live_attempt(&config).await {
            Ok(()) => return,
            Err(transient) => {
                assert!(
                    attempt < MAX_ATTEMPTS,
                    "subagent live test still hitting transient transport failures after \
                     {MAX_ATTEMPTS} attempts: {transient}"
                );
                eprintln!(
                    "subagent live test attempt {attempt}/{MAX_ATTEMPTS} hit transient \
                     transport failure, retrying: {transient}"
                );
            }
        }
    }
}
