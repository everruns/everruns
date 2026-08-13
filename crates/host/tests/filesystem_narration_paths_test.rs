// EVE-749: filesystem tool narration must use the active SessionFileSystem
// display_path contract so transcript paths match tool results.

#![cfg(feature = "filesystem")]

use async_trait::async_trait;
use everruns_core::MessageRetriever;
use everruns_core::atoms::{ActInput, AtomContext};
use everruns_core::capabilities::{
    CapabilityRegistry, SystemPromptContext, collect_capabilities_with_configs,
};
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::in_memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryMessageRetriever,
    InMemoryProviderStore,
};
use everruns_core::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_list_directory,
};
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ProviderStore, SessionFileSystem, SessionStore,
};
use everruns_core::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_core::{
    AgentCapabilityConfig, EventData, ExecutionSession, HarnessDefinition, MountFs,
    SessionExecutionState, ToolCall, WorkspaceRootSet,
};
use everruns_host::{
    InMemorySessionFileStore, RealDiskFileStore, ResolvedTurnInputs, RuntimeHostAdapter,
    execute_act_activity, multi_root_file_system,
};
use everruns_integrations_filesystem::FileSystemCapability;
use everruns_platform::SessionMutator;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
struct TestSessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, ExecutionSession>>>,
}

impl TestSessionStore {
    async fn insert(&self, session: ExecutionSession) {
        self.sessions.write().await.insert(session.id, session);
    }

    async fn set_status(
        &self,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> ExecutionSession {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.status = status;
        session.clone()
    }
}

#[async_trait]
impl SessionStore for TestSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_core::error::Result<Option<ExecutionSession>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

#[async_trait]
impl SessionMutator for TestSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> everruns_core::error::Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.title = Some(title);
        Ok(session.clone())
    }
}

#[derive(Clone)]
struct NarrationTestHost {
    capability_registry: CapabilityRegistry,
    harness_store: Arc<InMemoryHarnessStore>,
    session_store: Arc<TestSessionStore>,
    message_store: Arc<InMemoryMessageRetriever>,
    provider_store: Arc<InMemoryProviderStore>,
    event_emitter: Arc<InMemoryEventEmitter>,
    file_store: Arc<dyn SessionFileSystem>,
}

#[async_trait]
impl RuntimeHostAdapter for NarrationTestHost {
    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> everruns_core::error::Result<()> {
        self.session_store.set_status(session_id, status).await;
        Ok(())
    }

    async fn load_resolved_turn(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> everruns_core::error::Result<ResolvedTurnInputs> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .expect("session exists");
        let harness = self
            .harness_store
            .get_harness(session.harness_id)
            .await?
            .expect("harness exists");
        let snapshot = everruns_core::ResolvedExecutionSnapshot::project(&harness, None, &session)?;
        Ok(ResolvedTurnInputs {
            snapshot,
            messages: self.message_store.load(session_id).await?,
            mcp_tool_definitions: vec![],
        })
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        DriverRegistry::new()
    }

    fn harness_store(&self, _org_id: i64) -> Arc<dyn HarnessStore> {
        self.harness_store.clone()
    }

    fn agent_store(&self, _org_id: i64) -> Arc<dyn AgentStore> {
        Arc::new(InMemoryAgentStore::new())
    }

    fn session_store(&self, _org_id: i64) -> Arc<dyn SessionStore> {
        self.session_store.clone()
    }

    fn session_mutator(&self, _org_id: i64) -> Arc<dyn SessionMutator> {
        self.session_store.clone()
    }

    fn provider_store(&self, _org_id: i64) -> Arc<dyn ProviderStore> {
        self.provider_store.clone()
    }

    fn message_store(&self) -> Arc<dyn MessageRetriever> {
        self.message_store.clone()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    fn file_store(&self) -> Arc<dyn SessionFileSystem> {
        self.file_store.clone()
    }
}

fn harness() -> HarnessDefinition {
    HarnessDefinition {
        capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
        ..HarnessDefinition::new("files", "")
    }
}

fn session(session_id: SessionId, harness_id: HarnessId) -> ExecutionSession {
    ExecutionSession {
        id: session_id,
        workspace_id: everruns_core::WorkspaceId::from_uuid(session_id.uuid()),
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: None,
        title: None,
        goal: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        status: SessionExecutionState::Started,
        usage: None,
        parent_session_id: None,
        forked_from_session_id: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}

async fn build_tool_definitions(
    host: &NarrationTestHost,
    session_id: SessionId,
) -> Vec<everruns_core::ToolDefinition> {
    let ctx = SystemPromptContext::without_file_store(session_id);
    let collected = collect_capabilities_with_configs(
        &[AgentCapabilityConfig::new("session_file_system")],
        &host.capability_registry,
        &ctx,
    )
    .await;
    collected
        .tools
        .into_iter()
        .map(|tool| tool.to_definition())
        .collect()
}

struct NarrationEvents {
    started: String,
    completed: String,
    result_path: String,
}

async fn run_list_directory_act(
    host: &NarrationTestHost,
    session_id: SessionId,
    harness_id: HarnessId,
    path: Value,
) -> NarrationEvents {
    let tool_definitions = build_tool_definitions(host, session_id).await;
    let result = execute_act_activity(
        host,
        ActInput {
            org_id: Some(1),
            context: AtomContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                MessageId::from_uuid(Uuid::now_v7()),
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_list".into(),
                name: "list_directory".into(),
                arguments: json!({ "path": path }),
            }],
            tool_definitions,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .expect("act succeeds");

    assert_eq!(result.success_count, 1, "list_directory should succeed");
    let tool_result = &result.results[0].result;
    let result_path = tool_result
        .result
        .as_ref()
        .and_then(|value| value.get("path"))
        .and_then(|value| value.as_str())
        .expect("result path")
        .to_string();

    let events = host.event_emitter.events().await;
    let started = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolStarted(data) if data.tool_call.name == "list_directory" => {
                data.narration.clone()
            }
            _ => None,
        })
        .expect("tool.started narration");
    let completed = events
        .iter()
        .find_map(|event| match &event.data {
            EventData::ToolCompleted(data) if data.tool_name == "list_directory" => {
                data.narration.clone()
            }
            _ => None,
        })
        .expect("tool.completed narration");

    NarrationEvents {
        started,
        completed,
        result_path,
    }
}

fn host_with_store(file_store: Arc<dyn SessionFileSystem>) -> NarrationTestHost {
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.register(FileSystemCapability);
    NarrationTestHost {
        capability_registry,
        harness_store: Arc::new(InMemoryHarnessStore::new()),
        session_store: Arc::new(TestSessionStore::default()),
        message_store: Arc::new(InMemoryMessageRetriever::new()),
        provider_store: Arc::new(InMemoryProviderStore::new()),
        event_emitter: Arc::new(InMemoryEventEmitter::new()),
        file_store,
    }
}

#[tokio::test]
async fn mounted_real_disk_list_directory_narration_uses_workspace_paths() {
    let workspace = TempDir::new().unwrap();
    std::fs::create_dir_all(workspace.path().join("crates")).unwrap();
    let store = Arc::new(RealDiskFileStore::new(workspace.path()).expect("store"));
    let host_root = workspace.path().display().to_string();

    let host = host_with_store(store);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    host.harness_store.add_harness(harness_id, harness()).await;
    host.session_store
        .insert(session(session_id, harness_id))
        .await;

    let events =
        run_list_directory_act(&host, session_id, harness_id, json!("/workspace/crates")).await;

    assert_eq!(events.started, "Listing files in /workspace/crates");
    assert_eq!(events.completed, "Listed files in /workspace/crates");
    assert_eq!(events.result_path, "/workspace/crates");
    assert!(!events.started.contains(&host_root));
    assert!(!events.completed.contains(&host_root));
    assert!(!events.result_path.contains(&host_root));
}

#[tokio::test]
async fn mount_fs_list_directory_narration_matches_results() {
    let workspace = TempDir::new().unwrap();
    std::fs::create_dir_all(workspace.path().join("crates")).unwrap();
    let backend = Arc::new(RealDiskFileStore::new(workspace.path()).expect("store"));
    let store = MountFs::wrap(backend);
    let expected = store.display_path("crates");

    let host = host_with_store(store);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    host.harness_store.add_harness(harness_id, harness()).await;
    host.session_store
        .insert(session(session_id, harness_id))
        .await;

    let events = run_list_directory_act(&host, session_id, harness_id, json!("crates")).await;

    assert_eq!(events.started, format!("Listing files in {expected}"));
    assert_eq!(events.completed, format!("Listed files in {expected}"));
    assert_eq!(events.result_path, expected);
}

#[tokio::test]
async fn in_memory_list_directory_narration_keeps_workspace_alias() {
    let store = Arc::new(InMemorySessionFileStore::new());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    store
        .create_directory(session_id, "/crates")
        .await
        .expect("seed dir");

    let host = host_with_store(store);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    host.harness_store.add_harness(harness_id, harness()).await;
    host.session_store
        .insert(session(session_id, harness_id))
        .await;

    let events =
        run_list_directory_act(&host, session_id, harness_id, json!("/workspace/crates")).await;

    assert_eq!(events.started, "Listing files in /workspace/crates");
    assert_eq!(events.completed, "Listed files in /workspace/crates");
    assert_eq!(events.result_path, "/workspace/crates");
}

#[tokio::test]
async fn named_mount_narration_preserves_mount_path() {
    let session_id = SessionId::from_seed(749);
    let primary = TempDir::new().unwrap();
    let secondary = TempDir::new().unwrap();
    let roots = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), secondary.path().to_path_buf())],
    )
    .unwrap();
    let store = multi_root_file_system(&roots).expect("multi-root store");
    store
        .create_directory(session_id, "/workspace/roots/backend")
        .await
        .expect("seed mount dir");

    let mount_path = "/workspace/roots/backend";
    assert_eq!(
        store.display_path(mount_path),
        mount_path,
        "named mount display_path must preserve virtual mount identity"
    );
    assert!(
        store.is_mount_resolver(),
        "multi-root store must identify as mount resolver"
    );
    let wrapped = MountFs::wrap_if_needed(store.clone());
    let ctx = ToolNarrationContext::new(Some(wrapped.as_ref()));
    assert_eq!(
        narrate_list_directory(
            &json!({ "path": mount_path }),
            ToolNarrationPhase::Started,
            None,
            ctx,
        ),
        format!("Listing files in {mount_path}"),
        "helper narration must match mount display_path"
    );
    let host = host_with_store(store);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    host.harness_store.add_harness(harness_id, harness()).await;
    host.session_store
        .insert(session(session_id, harness_id))
        .await;

    let events = run_list_directory_act(&host, session_id, harness_id, json!(mount_path)).await;

    assert_eq!(events.started, format!("Listing files in {mount_path}"));
    assert_eq!(events.completed, format!("Listed files in {mount_path}"));
    assert_eq!(events.result_path, mount_path);
}

#[tokio::test]
async fn list_directory_root_inputs_narrate_display_root() {
    let workspace = TempDir::new().unwrap();
    let store = Arc::new(RealDiskFileStore::new(workspace.path()).expect("store"));
    let host_root = workspace.path().display().to_string();

    let host = host_with_store(store);
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    host.harness_store.add_harness(harness_id, harness()).await;
    host.session_store
        .insert(session(session_id, harness_id))
        .await;

    for path in [json!("/workspace"), json!("."), json!("/")] {
        let events = run_list_directory_act(&host, session_id, harness_id, path).await;
        assert_eq!(events.completed, "Listed files in /workspace");
        assert_eq!(events.result_path, "/workspace");
        assert!(!events.completed.contains(&host_root));
        assert!(!events.result_path.contains(&host_root));
    }
}
