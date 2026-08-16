// Composability: caller-supplied event bus + custom SessionFileSystemFactory
// are used by the runtime. Seam: an InProcessRuntime built from LocalBackends
// exposes the injected task registry / schedule store through the
// RuntimeHostAdapter, and a task lifecycle round-trips through the registry.
//
// Embedded smoke test: drive a real turn through a runtime wired from
// LocalBackends with an llmsim + a test math capability, proving the local
// stores compose into a working in-process runtime.

use everruns_host::HostComposition;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use everruns_core::events::Event;
use everruns_core::session_files::SessionFileSystem;
use everruns_core::session_task::{
    CreateSessionTask, SessionTaskState, SessionTaskUpdate, TASK_KIND_BACKGROUND_TOOL, TaskLinks,
    TaskWakePolicy,
};
use everruns_core::{CapabilityRegistry, InputMessage};
use everruns_host::{
    AgentBuilder, EventSink, EventSinkError, HarnessBuilder, HostBackends, InProcessRuntimeBuilder,
    RealDiskFileStore, RuntimeHostAdapter, SessionBuilder, SessionFileSystemFactory,
    SessionFileSystemFactoryContext,
};
use everruns_local::{LocalBackends, LocalProfile, SqliteDb};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::tool_types::ToolCall;
use everruns_test_support::{LlmSimConfig, LlmSimRuntimeExt, TestMathCapability};

// ---- Caller-supplied event bus decorator ----------------------------------

#[derive(Default)]
struct CountingEventSink {
    count: AtomicUsize,
}

impl EventSink for CountingEventSink {
    fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---- Custom SessionFileSystemFactory ---------------------------------------

#[derive(Debug)]
struct FlaggingFactory {
    root: std::path::PathBuf,
    used: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl SessionFileSystemFactory for FlaggingFactory {
    fn name(&self) -> &'static str {
        "FlaggingFactory"
    }
    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> everruns_provider::error::Result<Arc<dyn SessionFileSystem>> {
        self.used.store(true, Ordering::SeqCst);
        Ok(Arc::new(RealDiskFileStore::new(self.root.clone())?))
    }
}

fn ids() -> (
    everruns_provider::typed_id::HarnessId,
    everruns_provider::typed_id::AgentId,
    everruns_provider::typed_id::SessionId,
) {
    (
        "harness_00000000000000000000000000000091".parse().unwrap(),
        "agent_00000000000000000000000000000091".parse().unwrap(),
        "session_00000000000000000000000000000091".parse().unwrap(),
    )
}

#[tokio::test]
async fn embedder_bus_and_fs_factory_are_used() {
    let (harness_id, agent_id, session_id) = ids();
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();

    let sink = Arc::new(CountingEventSink::default());
    let count_handle = sink.clone();
    let fs_used = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let factory = Arc::new(FlaggingFactory {
        root: workspace.path().to_path_buf(),
        used: fs_used.clone(),
    });

    // Composable: pass our own bus on the runtime backends, our own FS factory
    // on the platform definition. LocalBackends only adds the local stores.
    let runtime_backends = HostBackends::in_memory().with_event_sink(sink);
    let profile = LocalProfile::new(data.path()).with_workspace_root(workspace.path());
    let local = LocalBackends::with_db(
        profile,
        runtime_backends,
        SqliteDb::open_in_memory().unwrap(),
    )
    .unwrap();

    let mut caps = CapabilityRegistry::new();
    caps.register(TestMathCapability);
    let platform = HostComposition::builder()
        .capability_registry(caps)
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(factory)
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .backends(local.runtime_backends.clone())
        .llm_sim_as_default(
            LlmSimConfig::fixed("Let me calculate.").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_mul".into(),
                    name: "multiply".into(),
                    arguments: serde_json::json!({"a": 6, "b": 7}),
                }],
                vec![],
            ]),
        )
        .harness(
            HarnessBuilder::new("math", "You are a math assistant.")
                .id(harness_id)
                .capability("test_math")
                .build(),
        )
        .agent(
            AgentBuilder::new("math-agent", "Use tools.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        )
        .build()
        .await
        .unwrap();

    // Seam: the adapter returns the injected stores (not None).
    assert!(
        runtime.session_task_registry().is_some(),
        "injected task registry must be visible via the adapter"
    );
    assert!(
        runtime.schedule_store(local.org_id()).is_some(),
        "injected schedule store must be visible via the adapter"
    );

    // Embedded smoke test: drive a real turn.
    let result = runtime
        .run_turn(session_id, InputMessage::user("What is 6 * 7?"))
        .await
        .unwrap();
    assert!(result.success, "turn should succeed");

    assert!(fs_used.load(Ordering::SeqCst), "custom FS factory was used");
    assert!(
        count_handle.count.load(Ordering::SeqCst) > 0,
        "caller sink observed turn events"
    );
}

#[tokio::test]
async fn seam_task_lifecycle_round_trips_through_injected_registry() {
    // Build the runtime via LocalBackends and exercise the registry returned by
    // the adapter — proving the same store the act path receives is reachable.
    let runtime_backends = HostBackends::in_memory();
    let local = LocalBackends::with_db(
        LocalProfile::default(),
        runtime_backends,
        SqliteDb::open_in_memory().unwrap(),
    )
    .unwrap();

    let (harness_id, agent_id, session_id) = ids();
    let mut caps = CapabilityRegistry::new();
    caps.register(TestMathCapability);
    let platform = HostComposition::new(caps, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .backends(local.runtime_backends.clone())
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .llm_sim_as_default(LlmSimConfig::fixed("ok"))
        .harness(HarnessBuilder::new("h", "p").id(harness_id).build())
        .agent(AgentBuilder::new("a", "p").id(agent_id).build())
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        )
        .build()
        .await
        .unwrap();

    let registry = runtime.session_task_registry().expect("registry injected");

    let task = registry
        .create(CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "seam task".to_string(),
            spec: serde_json::json!({}),
            state: SessionTaskState::Queued,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .unwrap();

    let done = registry
        .update(
            session_id,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                summary: Some("seam ok".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.state, SessionTaskState::Succeeded);

    // The schedule store from the adapter is the same SQLite-backed store.
    let schedule_store = runtime
        .schedule_store(local.org_id())
        .expect("schedule store");
    schedule_store
        .create_schedule(
            session_id,
            "seam schedule".to_string(),
            None,
            Some(chrono::Utc::now()),
            "UTC".to_string(),
        )
        .await
        .unwrap();
    assert_eq!(
        schedule_store
            .count_active_schedules(session_id)
            .await
            .unwrap(),
        1
    );
}
