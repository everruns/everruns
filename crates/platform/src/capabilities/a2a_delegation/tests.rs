//! Unit tests for outbound A2A agent delegation.

use super::*;
use a2a::StreamResponse;
use a2a::{AgentCapabilities, AgentInterface, Artifact, TaskStatus, TaskStatusUpdateEvent};
use a2a_server::agent_card::agent_card_router;
use a2a_server::{
    DefaultRequestHandler, InMemoryTaskStore, StaticAgentCard, jsonrpc::jsonrpc_router,
};
use axum::Router;
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_contracts::typed_id::SessionId;
use futures::stream;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use tokio::net::TcpListener;

/// Local mock agents need `allow_local_urls`; that hatch is gated to
/// `DEPLOYMENT_GRADE=dev`.
fn ensure_dev_deployment_grade() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        unsafe { std::env::set_var("DEPLOYMENT_GRADE", "dev") };
    });
}

#[derive(Default)]
struct TestStorageStore {
    values: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl everruns_core::session_services::SessionStorageStore for TestStorageStore {
    async fn set_value(&self, _session_id: SessionId, key: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn get_value(&self, _session_id: SessionId, key: &str) -> Result<Option<String>> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    async fn delete_value(&self, _session_id: SessionId, key: &str) -> Result<bool> {
        Ok(self.values.lock().unwrap().remove(key).is_some())
    }

    async fn list_keys(
        &self,
        _session_id: SessionId,
    ) -> Result<Vec<everruns_core::session_services::KeyInfo>> {
        let now = chrono::Utc::now();
        Ok(self
            .values
            .lock()
            .unwrap()
            .keys()
            .map(|key| everruns_core::session_services::KeyInfo {
                key: key.clone(),
                created_at: now,
                updated_at: now,
            })
            .collect())
    }

    async fn set_secret(&self, _session_id: SessionId, _name: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    async fn get_secret(&self, _session_id: SessionId, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }

    async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> Result<bool> {
        Ok(false)
    }

    async fn list_secrets(
        &self,
        _session_id: SessionId,
    ) -> Result<Vec<everruns_core::session_services::SecretInfo>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct TestFileStore {
    files: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl SessionFileSystem for TestFileStore {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(path)
            .map(|content| SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: Some(content.clone()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        _encoding: &str,
    ) -> Result<SessionFile> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), content.to_string());
        Ok(SessionFile {
            id: uuid::Uuid::new_v4(),
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: FileInfo::name_from_path(path),
            content: Some(content.to_string()),
            encoding: "text".to_string(),
            is_directory: false,
            is_readonly: false,
            size_bytes: content.len() as i64,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }

    async fn delete_file(
        &self,
        _session_id: SessionId,
        path: &str,
        _recursive: bool,
    ) -> Result<bool> {
        Ok(self.files.lock().unwrap().remove(path).is_some())
    }

    async fn list_directory(&self, _session_id: SessionId, _path: &str) -> Result<Vec<FileInfo>> {
        Ok(vec![])
    }

    async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
        Ok(None)
    }

    async fn grep_files(
        &self,
        _session_id: SessionId,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        Ok(vec![])
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        Ok(FileInfo {
            id: uuid::Uuid::new_v4(),
            session_id: session_id.uuid(),
            path: path.to_string(),
            name: FileInfo::name_from_path(path),
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }
}

struct EchoA2aExecutor;

impl a2a_server::AgentExecutor for EchoA2aExecutor {
    fn execute(
        &self,
        ctx: a2a_server::ExecutorContext,
    ) -> futures::stream::BoxStream<'static, std::result::Result<StreamResponse, a2a::A2AError>>
    {
        let task_id = ctx.task_id.clone();
        let context_id = ctx.context_id.clone();
        let text = ctx
            .message
            .as_ref()
            .and_then(Message::text)
            .unwrap_or_default()
            .to_string();
        let working = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task_id.clone(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: None,
                timestamp: None,
            },
            metadata: None,
        });
        let completed = StreamResponse::Task(Task {
            id: task_id,
            context_id,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            artifacts: Some(vec![Artifact {
                artifact_id: a2a::new_artifact_id(),
                name: Some("echo".to_string()),
                description: None,
                parts: vec![
                    Part::text(format!("echo: {text}")),
                    Part::data(json!({"echo": text})),
                ],
                metadata: None,
                extensions: None,
            }]),
            history: ctx.stored_task.and_then(|task| task.history),
            metadata: None,
        });
        Box::pin(stream::iter(vec![Ok(working), Ok(completed)]))
    }

    fn cancel(
        &self,
        ctx: a2a_server::ExecutorContext,
    ) -> futures::stream::BoxStream<'static, std::result::Result<StreamResponse, a2a::A2AError>>
    {
        let canceled = StreamResponse::Task(Task {
            id: ctx.task_id,
            context_id: ctx.context_id,
            status: TaskStatus {
                state: TaskState::Canceled,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
        });
        Box::pin(stream::once(async move { Ok(canceled) }))
    }
}

async fn spawn_real_a2a_agent() -> String {
    everruns_contracts::install_default_crypto_provider();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let card = AgentCard {
        name: "Echo A2A Agent".to_string(),
        description: "Real A2A test agent".to_string(),
        version: "1.0.0".to_string(),
        supported_interfaces: vec![AgentInterface::new(
            format!("{base_url}/jsonrpc"),
            "JSONRPC",
        )],
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(false),
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        skills: vec![],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    };
    let handler = Arc::new(DefaultRequestHandler::new(
        EchoA2aExecutor,
        InMemoryTaskStore::default(),
    ));
    let app = Router::new()
        .merge(agent_card_router(Arc::new(StaticAgentCard::new(card))))
        .nest("/jsonrpc", jsonrpc_router(handler));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    base_url
}

fn configured_capability(base_url: String) -> A2aDelegationConfig {
    // Local mock agents need the hatch; it is gated to DEPLOYMENT_GRADE=dev.
    ensure_dev_deployment_grade();
    A2aDelegationConfig {
        agents: vec![ExternalA2aAgentConfig {
            id: "echo".to_string(),
            name: "Echo".to_string(),
            description: Some("Echo test agent".to_string()),
            base_url: Some(base_url),
            agent_card: None,
            headers: BTreeMap::new(),
            preferred_binding: Some("JSONRPC".to_string()),
            poll_interval_ms: Some(100),
            allow_local_urls: true,
        }],
    }
}

fn context(storage_store: Arc<TestStorageStore>, file_store: Arc<TestFileStore>) -> ToolContext {
    ToolContext::with_stores(SessionId::new(), file_store, storage_store)
}

#[tokio::test]
async fn spawn_agent_foreground_calls_real_a2a_agent_with_target_id_alias() {
    let base_url = spawn_real_a2a_agent().await;
    let config = configured_capability(base_url);
    let tool = SpawnAgentTool::new(config);
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let ctx = context(storage_store, file_store);

    let result = tool
        .execute_with_context(
            json!({
                "instructions": "hello",
                "target": {"type": "external_a2a", "id": "echo"},
                "mode": "foreground",
                "wait_timeout_secs": 5
            }),
            &ctx,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success: {result:?}");
    };
    assert_eq!(value["status"], "completed");
    assert_eq!(value["result"], "echo: hello");
    assert!(value["result_path"].as_str().is_some());
}

#[tokio::test]
async fn spawn_agent_rejects_legacy_wait_mode() {
    let tool = SpawnAgentTool::new(configured_capability("http://127.0.0.1:1".to_string()));
    let ctx = context(
        Arc::new(TestStorageStore::default()),
        Arc::new(TestFileStore::default()),
    );
    let result = tool
        .execute_with_context(
            json!({
                "instructions": "never sent",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "mode": "wait"
            }),
            &ctx,
        )
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("expected legacy mode rejection: {result:?}");
    };
    assert!(message.contains("background, foreground"));
}

#[tokio::test]
async fn spawn_agent_foreground_validates_a2a_data_artifact_and_writes_task_result() {
    let tool = SpawnAgentTool::new(configured_capability(spawn_real_a2a_agent().await));
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let registry = Arc::new(InMemRegistry::default());
    let ctx =
        context(storage_store, file_store.clone()).with_session_task_registry(registry.clone());

    let result = tool
        .execute_with_context(
            json!({
                "instructions": "structured",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "mode": "foreground",
                "wait_timeout_secs": 5,
                "result_schema": {
                    "type": "object",
                    "properties": {"echo": {"type": "string"}},
                    "required": ["echo"],
                    "additionalProperties": false
                }
            }),
            &ctx,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success: {result:?}");
    };
    assert_eq!(value["status"], "completed");
    let task_id = value["task_id"].as_str().expect("task_id");
    let expected_path = everruns_core::session_task::task_result_path(task_id);
    assert_eq!(value["result_path"], expected_path);
    let content = file_store
        .files
        .lock()
        .unwrap()
        .get(&expected_path)
        .cloned()
        .expect("result file");
    assert_eq!(
        serde_json::from_str::<Value>(&content).unwrap(),
        json!({"echo": "structured"})
    );
    let task = registry
        .get(ctx.session_id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(task.result_path.as_deref(), Some(expected_path.as_str()));
}

#[tokio::test]
async fn spawn_agent_foreground_marks_a2a_schema_mismatch_failed() {
    let tool = SpawnAgentTool::new(configured_capability(spawn_real_a2a_agent().await));
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let registry = Arc::new(InMemRegistry::default());
    let ctx = context(storage_store, file_store).with_session_task_registry(registry.clone());

    let result = tool
        .execute_with_context(
            json!({
                "instructions": "structured",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "mode": "foreground",
                "wait_timeout_secs": 5,
                "result_schema": {
                    "type": "object",
                    "properties": {"echo": {"type": "integer"}},
                    "required": ["echo"]
                }
            }),
            &ctx,
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("expected terminal run result: {result:?}");
    };
    assert_eq!(value["status"], "failed");
    let task_id = value["task_id"].as_str().expect("task_id");
    let task = registry
        .get(ctx.session_id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, SessionTaskState::Failed);
    assert_eq!(
        task.error.as_ref().map(|error| error.kind.as_str()),
        Some("schema_mismatch")
    );
    assert!(task.result_path.is_none());
}

#[tokio::test]
async fn spawn_agent_rejects_message_schema_for_external_a2a() {
    let tool = SpawnAgentTool::new(configured_capability("http://127.0.0.1:1".to_string()));
    let ctx = context(
        Arc::new(TestStorageStore::default()),
        Arc::new(TestFileStore::default()),
    );
    let result = tool
        .execute_with_context(
            json!({
                "instructions": "never sent",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "message_schema": {"type": "object"}
            }),
            &ctx,
        )
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("expected explicit rejection: {result:?}");
    };
    assert!(message.contains("message_schema is not supported for external_a2a"));
}

#[test]
fn uk_localization_and_schema_one_of_match_validation() {
    let cap = A2aAgentDelegationCapability;
    assert_eq!(cap.localized_name(Some("uk-UA")), "Делегування агентам A2A");
    assert!(
        cap.localized_description(Some("uk-UA"))
            .contains("Делегує роботу")
    );
    assert!(cap.describe_schema(Some("uk")).is_some());
    assert!(cap.describe_schema(None).is_some());

    // preferred_binding oneOf consts must be exactly the values
    // validate_config accepts.
    let schema = cap.config_schema().expect("config schema");
    let consts: Vec<&str> =
        schema["properties"]["agents"]["items"]["properties"]["preferred_binding"]["oneOf"]
            .as_array()
            .expect("oneOf")
            .iter()
            .map(|v| v["const"].as_str().expect("const"))
            .collect();
    assert_eq!(consts, vec!["JSONRPC", "HTTP+JSON"]);
    for binding in consts {
        let config = json!({
            "agents": [{
                "id": "echo",
                "name": "Echo",
                "base_url": "https://agent.example.com",
                "preferred_binding": binding
            }]
        });
        cap.validate_config(&config)
            .unwrap_or_else(|e| panic!("{binding} should validate: {e}"));
    }
    assert!(
        cap.validate_config(&json!({
            "agents": [{
                "id": "echo",
                "name": "Echo",
                "base_url": "https://agent.example.com",
                "preferred_binding": "SMTP"
            }]
        }))
        .is_err()
    );

    let tool_schema = SpawnAgentTool::new(A2aDelegationConfig::default()).parameters_schema();
    assert_eq!(
        tool_schema["properties"]["mode"]["enum"],
        json!(["background", "foreground"])
    );
    assert!(tool_schema["properties"]["target"]["properties"]["id"].is_object());
}

#[test]
fn validates_local_urls_only_with_escape_hatch() {
    let mut config = configured_capability("http://127.0.0.1:1".to_string());
    config.agents[0].allow_local_urls = false;
    assert!(config.agents[0].validate().is_err());
    config.agents[0].allow_local_urls = true;
    assert!(config.agents[0].validate().is_ok());
}

#[test]
fn reattach_network_access_prefers_persisted_run_policy() {
    use everruns_contracts::typed_id::SessionId;

    let config = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: Some("https://allowed.example.com/a2a".to_string()),
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };
    let mut record = AgentRunRecord::new(
        "run-policy".to_string(),
        &config,
        "instructions".to_string(),
        SpawnMode::Background,
        false,
        None,
    );
    let persisted_policy = NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]);
    record.network_access = Some(persisted_policy.clone());

    let reaper_context = ToolContext::new(SessionId::new()).with_network_access(None);
    assert_eq!(
        reattach_network_access(&record, &reaper_context),
        Some(persisted_policy.clone())
    );

    let fallback_policy = NetworkAccessList::allow_only(vec!["fallback.example.com".to_string()]);
    let fallback_context =
        ToolContext::new(SessionId::new()).with_network_access(Some(fallback_policy.clone()));
    record.network_access = None;
    assert_eq!(
        reattach_network_access(&record, &fallback_context),
        Some(fallback_policy)
    );
}

#[test]
fn enforce_network_access_blocks_disallowed_base_url() {
    use everruns_core::network_access::NetworkAccessList;
    use everruns_contracts::typed_id::SessionId;

    let agent = ExternalA2aAgentConfig {
        id: "a".to_string(),
        name: "a".to_string(),
        description: None,
        base_url: Some("https://blocked.example.com".to_string()),
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };
    let card = AgentCard {
        name: "a".to_string(),
        description: "a".to_string(),
        version: "1".to_string(),
        supported_interfaces: vec![],
        capabilities: AgentCapabilities {
            streaming: None,
            push_notifications: None,
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec![],
        default_output_modes: vec![],
        skills: vec![],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    };

    let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
        NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
    ));
    let err = enforce_network_access_pre_resolve(&agent, &ctx).unwrap_err();
    assert!(
        err.contains("blocked.example.com"),
        "unexpected error: {err}"
    );

    let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
        NetworkAccessList::allow_only(vec!["blocked.example.com".to_string()]),
    ));
    enforce_network_access_pre_resolve(&agent, &ctx).unwrap();
    enforce_network_access_post_resolve(&card, &ctx).unwrap();
}

#[test]
fn enforce_network_access_blocks_disallowed_interface_url() {
    use everruns_core::network_access::NetworkAccessList;
    use everruns_contracts::typed_id::SessionId;

    let card = AgentCard {
        name: "a".to_string(),
        description: "a".to_string(),
        version: "1".to_string(),
        supported_interfaces: vec![AgentInterface::new(
            "https://probe.internal/api".to_string(),
            "JSONRPC",
        )],
        capabilities: AgentCapabilities {
            streaming: None,
            push_notifications: None,
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec![],
        default_output_modes: vec![],
        skills: vec![],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    };
    let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
        NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
    ));
    let err = enforce_network_access_post_resolve(&card, &ctx).unwrap_err();
    assert!(err.contains("probe.internal"), "unexpected error: {err}");
}

#[test]
fn enforce_network_access_pre_resolve_skips_when_inline_card_present() {
    use everruns_core::network_access::NetworkAccessList;
    use everruns_contracts::typed_id::SessionId;

    // base_url not on the allowlist, but agent_card is supplied inline so
    // resolve_card never performs discovery against base_url. The pre-resolve
    // gate must skip the base_url check to avoid spurious failures.
    let inline_card = AgentCard {
        name: "a".to_string(),
        description: "a".to_string(),
        version: "1".to_string(),
        supported_interfaces: vec![AgentInterface::new(
            "https://allowed.example.com/api".to_string(),
            "JSONRPC",
        )],
        capabilities: AgentCapabilities {
            streaming: None,
            push_notifications: None,
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec![],
        default_output_modes: vec![],
        skills: vec![],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    };
    let agent = ExternalA2aAgentConfig {
        id: "a".to_string(),
        name: "a".to_string(),
        description: None,
        base_url: Some("https://stale.unused.example.com".to_string()),
        agent_card: Some(inline_card),
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };
    let ctx = ToolContext::new(SessionId::new()).with_network_access(Some(
        NetworkAccessList::allow_only(vec!["allowed.example.com".to_string()]),
    ));
    enforce_network_access_pre_resolve(&agent, &ctx).unwrap();
}

#[test]
fn local_url_escape_hatch_still_rejects_bad_url_shape() {
    let mut config = configured_capability("file:///tmp/agent".to_string());
    config.agents[0].allow_local_urls = true;
    assert!(config.agents[0].validate().is_err());

    let mut config = configured_capability("http://127.0.0.1:1".to_string());
    config.agents[0].allow_local_urls = true;
    config.agents[0].preferred_binding = Some("SMTP".to_string());
    assert!(config.agents[0].validate().is_err());

    config.agents[0].preferred_binding = Some("JSONRPC".to_string());
    config.agents[0].poll_interval_ms = Some(99);
    assert!(config.agents[0].validate().is_err());
}

// -------------------------------------------------------------------------
// ExternalAgentTaskExecutor::start() tests
// -------------------------------------------------------------------------

/// Minimal in-memory task registry for executor tests.
#[derive(Default, Clone)]
struct InMemRegistry {
    tasks: Arc<Mutex<HashMap<String, everruns_core::session_task::SessionTask>>>,
}

#[async_trait]
impl everruns_core::session_task::SessionTaskRegistry for InMemRegistry {
    async fn create(
        &self,
        input: everruns_core::session_task::CreateSessionTask,
    ) -> everruns_contracts::error::Result<everruns_core::session_task::SessionTask> {
        let task = everruns_core::session_task::new_session_task(input, chrono::Utc::now());
        self.tasks
            .lock()
            .unwrap()
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn get(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        task_id: &str,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        Ok(self.tasks.lock().unwrap().get(task_id).cloned())
    }

    async fn list(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _filter: Option<&everruns_core::session_task::SessionTaskFilter>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_task::SessionTask>> {
        Ok(self.tasks.lock().unwrap().values().cloned().collect())
    }

    async fn update(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        task_id: &str,
        update: everruns_core::session_task::SessionTaskUpdate,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(task) = tasks.get_mut(task_id) else {
            return Ok(None);
        };
        everruns_core::session_task::apply_task_update(task, update, chrono::Utc::now());
        Ok(Some(task.clone()))
    }

    async fn request_cancel(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _task_id: &str,
    ) -> everruns_contracts::error::Result<Option<everruns_core::session_task::SessionTask>> {
        Ok(None)
    }

    async fn record_message(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _task_id: &str,
        _message: everruns_core::session_task::NewTaskMessage,
    ) -> everruns_contracts::error::Result<everruns_core::session_task::TaskMessage> {
        Err(everruns_contracts::error::AgentLoopError::tool(
            "not implemented",
        ))
    }

    async fn list_messages(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _task_id: &str,
        _limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> everruns_contracts::error::Result<Vec<everruns_core::session_task::TaskMessage>> {
        Ok(vec![])
    }
}

#[test]
fn a2a_delegation_depends_on_session_tasks() {
    let cap = A2aAgentDelegationCapability;

    assert_eq!(cap.dependencies(), vec![SESSION_TASKS_CAPABILITY_ID]);
}

#[tokio::test]
async fn background_spawn_requires_task_registry_before_remote_work() {
    let config = configured_capability("http://127.0.0.1:1".to_string());
    let spawn = SpawnAgentTool::new(config);
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let ctx = context(storage_store.clone(), file_store);

    let result = spawn
        .execute_with_context(
            json!({
                "instructions": "background",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "mode": "background"
            }),
            &ctx,
        )
        .await;

    let ToolExecutionResult::ToolError(message) = result else {
        panic!("background spawn should reject missing task registry");
    };
    assert!(
        message.contains("requires session_task_registry"),
        "unexpected error: {message}"
    );
    assert!(
        storage_store.values.lock().unwrap().is_empty(),
        "background spawn must not persist or launch remote work without task tracking"
    );
}

/// Parity check for the retired `wait_agent`: a background `spawn_agent`
/// run is observable end-to-end through the generic `wait_task` tool.
/// The background poll loop mirrors each remote snapshot onto the session
/// task via `save_run`, so `wait_task` converges to the terminal state.
#[tokio::test]
async fn background_spawn_is_waitable_via_generic_wait_task() {
    use crate::capabilities::session_tasks::WaitTaskTool;

    let base_url = spawn_real_a2a_agent().await;
    let config = configured_capability(base_url);
    let spawn = SpawnAgentTool::new(config);

    // A context WITH a task registry so the background run creates and
    // mirrors a session task (the registry is what wait_task reads).
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let registry = Arc::new(InMemRegistry::default());
    let ctx = ToolContext::with_stores(SessionId::new(), file_store, storage_store)
        .with_session_task_registry(registry.clone());

    let result = spawn
        .execute_with_context(
            json!({
                "instructions": "background",
                "target": {"type": "external_a2a", "external_agent_id": "echo"},
                "mode": "background",
                "wait_timeout_secs": 5,
                "wake_on_completion": false
            }),
            &ctx,
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected spawn success: {result:?}");
    };
    let task_id = value["task_id"]
        .as_str()
        .expect("background spawn returns a task_id when a registry is present");

    let waited = WaitTaskTool
        .execute_with_context(json!({"task_id": task_id, "timeout_seconds": 5}), &ctx)
        .await;
    let ToolExecutionResult::Success(value) = waited else {
        panic!("expected wait_task success: {waited:?}");
    };
    assert_eq!(
        value["timed_out"], false,
        "wait_task should observe a terminal state, got {value:?}"
    );
    assert_eq!(value["task"]["state"], "succeeded");
}

/// Build a SessionTask snapshot for testing (not persisted in any store).
fn fake_task_with_spec(
    session_id: everruns_contracts::typed_id::SessionId,
    run_id: &str,
    attempt: i32,
) -> everruns_core::session_task::SessionTask {
    let now = chrono::Utc::now();
    everruns_core::session_task::SessionTask {
        id: format!("task_{run_id}"),
        session_id,
        root_session_id: None,
        kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
        display_name: "Test external agent".to_string(),
        spec: json!({ "run_id": run_id }),
        state: SessionTaskState::Running,
        state_detail: None,
        progress: None,
        links: TaskLinks::default(),
        wake_policy: TaskWakePolicy::Silent,
        input_request: None,
        cancel_requested_at: None,
        summary: None,
        result_path: None,
        artifacts: vec![],
        error: None,
        attempt,
        worker_id: None,
        heartbeat_at: None,
        started_at: None,
        finished_at: None,
        created_at: now,
        updated_at: now,
    }
}

/// start() with a terminal run record should mirror state and return Ok.
#[tokio::test]
async fn external_agent_executor_start_mirrors_terminal_run() {
    let storage = Arc::new(TestStorageStore::default());
    let registry = Arc::new(InMemRegistry::default());
    let session_id = everruns_contracts::typed_id::SessionId::new();

    let run_id = "run-terminal".to_string();
    let config = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: None,
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };
    let task_id = format!("task_{run_id}");

    // Create a completed run record.
    let mut record = AgentRunRecord::new(
        run_id.clone(),
        &config,
        "instructions".to_string(),
        SpawnMode::Background,
        false,
        None,
    );
    record.status = AgentRunStatus::Completed;
    record.result = Some("done".to_string());
    record.remote_task_id = Some("remote-xyz".to_string());
    record.task_id = Some(task_id.clone());

    // Persist the run record.
    let serialized = serde_json::to_string(&record).unwrap();
    storage
        .set_value(session_id, &run_key(&run_id), &serialized)
        .await
        .unwrap();

    // Create the session task in the registry so mirror_run_to_task can update it.
    registry
        .create(everruns_core::session_task::CreateSessionTask {
            session_id,
            id: Some(task_id.clone()),
            kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
            display_name: "Echo".to_string(),
            spec: json!({ "run_id": &run_id }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .unwrap();

    let ctx = ToolContext::new(session_id)
        .with_storage_store_arc(
            storage.clone() as Arc<dyn everruns_core::session_services::SessionStorageStore>
        )
        .with_session_task_registry(registry.clone());

    let task = fake_task_with_spec(session_id, &run_id, 2);
    let executor = ExternalAgentTaskExecutor;
    executor
        .start(&task, &ctx)
        .await
        .expect("start should succeed");

    // The registry task should now reflect the terminal state.
    let updated = registry.get(session_id, &task_id).await.unwrap().unwrap();
    assert_eq!(
        updated.state,
        SessionTaskState::Succeeded,
        "terminal run should mirror to succeeded"
    );
}

/// A heartbeat fence miss (task attempt moved past ours) must abort the
/// poll loop with the superseded error before any remote call or write.
#[tokio::test]
async fn wait_for_run_exits_superseded_on_fence_miss() {
    let storage = Arc::new(TestStorageStore::default());
    let registry = Arc::new(InMemRegistry::default());
    let session_id = everruns_contracts::typed_id::SessionId::new();

    let run_id = "run-superseded".to_string();
    // Inline card with a non-routable host: the heartbeat fence check
    // fires before build_client / DNS pinning / any remote get_task call.
    let inline_card = AgentCard {
        name: "Echo".to_string(),
        description: "Echo".to_string(),
        version: "1".to_string(),
        supported_interfaces: vec![AgentInterface::new(
            "https://agent.example.com/api".to_string(),
            "JSONRPC",
        )],
        capabilities: AgentCapabilities {
            streaming: None,
            push_notifications: None,
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec![],
        default_output_modes: vec![],
        skills: vec![],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    };
    let config = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: Some("https://agent.example.com".to_string()),
        agent_card: Some(inline_card),
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };
    let task_id = format!("task_{run_id}");

    let mut record = AgentRunRecord::new(
        run_id.clone(),
        &config,
        "instructions".to_string(),
        SpawnMode::Background,
        false,
        None,
    );
    record.remote_task_id = Some("remote-xyz".to_string());
    record.task_id = Some(task_id.clone());

    // Task in the registry already at attempt 2 (reaper superseded us).
    registry
        .create(everruns_core::session_task::CreateSessionTask {
            session_id,
            id: Some(task_id.clone()),
            kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
            display_name: "Echo".to_string(),
            spec: json!({ "run_id": &run_id }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .unwrap();
    registry
        .update(
            session_id,
            &task_id,
            everruns_core::session_task::SessionTaskUpdate {
                increment_attempt: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let ctx = ToolContext::new(session_id)
        .with_storage_store_arc(
            storage.clone() as Arc<dyn everruns_core::session_services::SessionStorageStore>
        )
        .with_session_task_registry(registry.clone());

    // Poll as the old executor (attempt 1): the first heartbeat reveals
    // the supersession and the loop exits before any remote call.
    let outcome = wait_for_run(&ctx, &config, record, 30, Some(1))
        .await
        .expect("superseded poll must not be a transport error");
    // EVE-645: supersession is now a typed WaitOutcome variant rather than
    // a string-prefix sniff on the error.
    let WaitOutcome::Superseded {
        run_id: superseded_run_id,
        attempt,
        by_attempt,
    } = outcome
    else {
        panic!("expected Superseded outcome");
    };
    assert_eq!(superseded_run_id, run_id);
    assert_eq!(attempt, 1);
    assert_eq!(by_attempt, 2);
    // Diagnostic string still carries the legacy prefix for log greps.
    assert!(
        WaitOutcome::superseded_message(&superseded_run_id, attempt, by_attempt)
            .starts_with(SUPERSEDED_ERROR_PREFIX)
    );
}

// EVE-645: timeout is selected via the typed WaitOutcome::TimedOut variant
// and its message stays byte-identical to the legacy string.
#[test]
fn wait_outcome_timed_out_message_is_stable() {
    let msg = WaitOutcome::timed_out_message("run-123", 30);
    assert_eq!(
        msg,
        "Timed out waiting for external agent run run-123 after 30s"
    );
}

/// start() with a run that has no remote_task_id should return an error.
#[tokio::test]
async fn external_agent_executor_start_errors_on_missing_remote_task_id() {
    let storage = Arc::new(TestStorageStore::default());
    let session_id = everruns_contracts::typed_id::SessionId::new();

    let run_id = "run-no-remote".to_string();
    let config = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: None,
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: false,
    };

    // Create a run record without a remote_task_id (never reached the remote agent).
    let record = AgentRunRecord::new(
        run_id.clone(),
        &config,
        "instructions".to_string(),
        SpawnMode::Background,
        false,
        None,
    );
    assert!(record.remote_task_id.is_none());

    let serialized = serde_json::to_string(&record).unwrap();
    storage
        .set_value(session_id, &run_key(&run_id), &serialized)
        .await
        .unwrap();

    let ctx = ToolContext::new(session_id).with_storage_store_arc(
        storage.clone() as Arc<dyn everruns_core::session_services::SessionStorageStore>
    );

    let task = fake_task_with_spec(session_id, &run_id, 2);
    let executor = ExternalAgentTaskExecutor;
    let result = executor.start(&task, &ctx).await;
    assert!(
        result.is_err(),
        "start() should error when remote_task_id is absent"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("remote_task_id"),
        "error should mention remote_task_id: {err}"
    );
}

/// Legacy records written before the task-to-instructions rename and the
/// A2A wait-to-foreground mode rename should still load from durable
/// session storage.
#[test]
fn agent_run_record_accepts_legacy_task_field() {
    let legacy = json!({
        "run_id": "legacy-run",
        "kind": "external_a2a",
        "external_agent_id": "echo",
        "external_agent_name": "Echo",
        "task": "legacy instructions",
        "mode": "wait",
        "status": "submitted"
    });

    let record: AgentRunRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.instructions, "legacy instructions");
    assert_eq!(record.mode, SpawnMode::Foreground);

    let serialized = serde_json::to_value(&record).unwrap();
    assert_eq!(serialized["instructions"], "legacy instructions");
    assert_eq!(serialized["mode"], "foreground");
    assert!(serialized.get("task").is_none());
}

/// Roundtrip: save_run → load_run → load_run_for_task all resolve consistently.
#[tokio::test]
async fn agent_run_storage_roundtrip() {
    let storage_store = Arc::new(TestStorageStore::default());
    let file_store = Arc::new(TestFileStore::default());
    let ctx = context(storage_store, file_store);

    let run_id = "test-run-roundtrip".to_string();
    let task_id = "task-abc".to_string();
    let config = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: Some("http://localhost:1".to_string()),
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: true,
    };
    let mut record = AgentRunRecord::new(
        run_id.clone(),
        &config,
        "do something".to_string(),
        SpawnMode::Foreground,
        false,
        None,
    );
    record.task_id = Some(task_id.clone());

    // save_run then load_run should round-trip the record.
    save_run(&ctx, &record).await.expect("save_run failed");
    let loaded = load_run(&ctx, &run_id).await.expect("load_run failed");
    assert_eq!(loaded.run_id, run_id);
    assert_eq!(loaded.task_id.as_deref(), Some(task_id.as_str()));

    // load_run_for_task with run_id in spec should resolve the same record.
    let now = chrono::Utc::now();
    let fake_task = SessionTask {
        id: task_id.clone(),
        session_id: ctx.session_id,
        root_session_id: None,
        kind: TASK_KIND_EXTERNAL_AGENT.to_string(),
        display_name: "Echo".to_string(),
        spec: json!({ "run_id": &run_id }),
        state: SessionTaskState::Running,
        state_detail: None,
        progress: None,
        links: TaskLinks::default(),
        wake_policy: TaskWakePolicy::Silent,
        input_request: None,
        cancel_requested_at: None,
        summary: None,
        result_path: None,
        artifacts: vec![],
        error: None,
        attempt: 1,
        worker_id: None,
        heartbeat_at: None,
        started_at: None,
        finished_at: None,
        created_at: now,
        updated_at: now,
    };
    let from_task = load_run_for_task(&ctx, &fake_task)
        .await
        .expect("load_run_for_task failed");
    assert_eq!(from_task.run_id, run_id);

    // The prefix-derived listing should include the run_id.
    let storage = ctx.storage_store.as_ref().unwrap();
    let index = list_run_ids(storage.as_ref(), ctx.session_id).await;
    assert!(
        index.contains(&run_id),
        "run_id not found in index: {index:?}"
    );
}
