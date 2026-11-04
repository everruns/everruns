//! Host capability, provider, recovery and observer services composed into
//! Engine-owned runtimes (EVE-1296). Imports only the public `everruns` API.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use everruns::hosting::services::{
    AgentLoopError, Capability, CommandDescriptor, CommandSource, DriverRegistry, HostEvent,
    HostEventListener, HostModelId, HostModelSpec, HostProviderKey, HostProviders, HostServices,
    LlmRetryConfig, ModelViewContext, ModelViewProvider, PolicyGate, PreToolUseDecision,
    PreToolUseHook, ProviderConfig, ProviderMetadata, ProviderStore, RuntimeMessage,
    SessionFileSystem, Tool, ToolCall, ToolContext, ToolDefinition, ToolExecutionResult,
    async_trait,
};
use everruns::llm::Message;
use everruns::{
    Agent, BuildError, ChatDriver, DriverConfig, DriverId, Engine, EventListener, LlmCallConfig,
    LlmResponseStream, LlmSimConfig, LlmStreamEvent, Model, Provider, ProviderEndpoint,
    SessionEvent,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// A host capability: one side-effecting tool, a policy guard, a model view and
// a slash command.
// ---------------------------------------------------------------------------

const GUARD_REASON: &str = "host policy forbids writes outside notes/";
const VIEW_MARKER: &str = "HOST_MODEL_VIEW_MARKER";
const INSTRUCTIONS: &str = "HOST_CAPABILITY_INSTRUCTIONS: keep writes under notes/.";

#[derive(Clone, Default)]
struct Effects(Arc<Mutex<Vec<String>>>);

impl Effects {
    fn paths(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct RecordWrite {
    effects: Effects,
}

#[async_trait]
impl Tool for RecordWrite {
    fn name(&self) -> &str {
        "record_write"
    }

    fn description(&self) -> &str {
        "Write a note into the session workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "text": { "type": "string" }
            },
            "required": ["path", "text"]
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::internal_error_msg("record_write needs a session filesystem")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = arguments["path"].as_str().unwrap_or_default().to_string();
        let text = arguments["text"].as_str().unwrap_or_default();
        self.effects.0.lock().unwrap().push(path.clone());
        let Some(files) = &context.file_store else {
            return ToolExecutionResult::internal_error_msg("no session filesystem");
        };
        match files
            .write_file(context.workspace_fs_key(), &path, text, "text")
            .await
        {
            Ok(_) => ToolExecutionResult::success(json!({ "written": path })),
            Err(error) => ToolExecutionResult::tool_error(error.to_string()),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

struct WorkOnly;

#[async_trait]
impl PreToolUseHook for WorkOnly {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _definition: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        let path = tool_call.arguments["path"].as_str().unwrap_or_default();
        if tool_call.name == "record_write" && !path.starts_with("notes/") {
            return PreToolUseDecision::Block {
                tool_call,
                reason: GUARD_REASON.into(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

struct MarkerView;

impl ModelViewProvider for MarkerView {
    fn apply_model_view(
        &self,
        mut messages: Vec<RuntimeMessage>,
        _config: &Value,
        _context: &ModelViewContext<'_>,
    ) -> Vec<RuntimeMessage> {
        messages.push(RuntimeMessage::user(VIEW_MARKER));
        messages
    }
}

struct HostPolicy {
    effects: Effects,
}

impl Capability for HostPolicy {
    fn id(&self) -> &str {
        "host_policy"
    }

    fn name(&self) -> &str {
        "Host policy"
    }

    fn description(&self) -> &str {
        "Workspace writes with a host guard."
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(INSTRUCTIONS)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(RecordWrite {
            effects: self.effects.clone(),
        })]
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(PolicyGate(WorkOnly))]
    }

    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        Some(Arc::new(MarkerView))
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "policy".into(),
            description: "Show the host write policy.".into(),
            source: CommandSource::System,
            args: Vec::new(),
        }]
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

type Captured = Arc<Mutex<Vec<Vec<Message>>>>;

fn scripted(calls: Vec<Vec<ToolCall>>) -> (Model, Captured) {
    let captured: Captured = Arc::default();
    let mut config = LlmSimConfig::fixed("done").with_tool_call_sequence(calls);
    config.message_capture = Some(captured.clone());
    (Model::simulated_with_config(config), captured)
}

fn text_of(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| message.content.to_text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn host_capability_guards_views_and_contributes_through_an_engine_session() {
    let workspace = tempfile::tempdir().unwrap();
    let effects = Effects::default();
    let (model, captured) = scripted(vec![
        vec![
            call(
                "allowed",
                "record_write",
                json!({"path": "notes/a.txt", "text": "kept"}),
            ),
            call(
                "denied",
                "record_write",
                json!({"path": "etc/passwd", "text": "nope"}),
            ),
        ],
        vec![],
    ]);
    let agent = Agent::builder()
        .instructions("Write notes when asked.")
        .model(model)
        .workspace(workspace.path())
        .workspace_policy(everruns::WorkspacePolicy::read_write())
        .host_services(HostServices::new().capability(HostPolicy {
            effects: effects.clone(),
        }))
        .build()
        .unwrap();
    let session = Engine::new().create(agent);

    let context = session.inspect().await.unwrap();
    let tool = context
        .tools
        .iter()
        .find(|tool| tool.name == "record_write")
        .expect("host tool is model-visible");
    assert_eq!(tool.description, "Write a note into the session workspace.");
    assert_eq!(tool.parameters["required"], json!(["path", "text"]));
    assert!(context.instructions.contains(INSTRUCTIONS));
    assert!(
        context
            .commands
            .iter()
            .any(|command| command.name == "policy"),
        "commands: {:?}",
        context.commands
    );

    let turn = session.run("write the notes").await.unwrap();
    assert!(turn.success, "{:?}", turn.error);

    // The guard ran before dispatch: the denied call had no side effect.
    assert_eq!(effects.paths(), vec!["notes/a.txt".to_string()]);
    let calls = captured.lock().unwrap().clone();
    // The allowed call wrote through the Environment-selected workspace.
    let written = std::fs::read_to_string(workspace.path().join("notes/a.txt"));
    assert_eq!(
        written.as_deref().ok(),
        Some("kept"),
        "tool results seen by the model: {}",
        calls
            .last()
            .map(|messages| text_of(messages))
            .unwrap_or_default()
    );
    assert!(!workspace.path().join("etc/passwd").exists());

    assert_eq!(calls.len(), 2, "one call per reasoning iteration");
    for messages in &calls {
        assert!(
            text_of(messages).contains(VIEW_MARKER),
            "the model view reached every provider call"
        );
    }
    assert!(
        text_of(&calls[1]).contains(GUARD_REASON),
        "the model sees why the call was blocked"
    );
}

#[tokio::test]
async fn host_capability_writes_stay_inside_the_environment_workspace() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let effects = Effects::default();
    let (model, captured) = scripted(vec![
        vec![call(
            "escape",
            "record_write",
            json!({"path": "notes/../../escape.txt", "text": "outside"}),
        )],
        vec![],
    ]);
    let agent = Agent::builder()
        .instructions("Write notes when asked.")
        .model(model)
        .workspace(&workspace)
        // Writes are allowed, so only containment can refuse the escape.
        .workspace_policy(everruns::WorkspacePolicy::read_write())
        .host_services(HostServices::new().capability(HostPolicy {
            effects: effects.clone(),
        }))
        .build()
        .unwrap();

    let turn = Engine::new().create(agent).run("escape").await.unwrap();
    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(effects.paths().len(), 1, "the host tool itself ran");
    // The session filesystem either refuses the traversal or clamps it to the
    // workspace root; nothing lands beside the workspace.
    assert!(!root.path().join("escape.txt").exists());
    let calls = captured.lock().unwrap().clone();
    let reply = text_of(&calls[1]);
    assert!(
        !reply.contains("\"written\"") || workspace.join("escape.txt").exists(),
        "a reported write must be inside the workspace: {reply}"
    );
}

#[tokio::test]
async fn registered_host_capabilities_activate_by_reference() {
    let agent = Agent::builder()
        .instructions("Use host tools.")
        .model(Model::simulated("ok"))
        .capability("host_policy")
        .host_services(HostServices::new().register(HostPolicy {
            effects: Effects::default(),
        }))
        .build()
        .unwrap();
    let context = Engine::new().create(agent).inspect().await.unwrap();
    assert!(context.tools.iter().any(|tool| tool.name == "record_write"));

    let dormant = Agent::builder()
        .instructions("Use host tools.")
        .model(Model::simulated("ok"))
        .host_services(HostServices::new().register(HostPolicy {
            effects: Effects::default(),
        }))
        .build()
        .unwrap();
    let context = Engine::new().create(dormant).inspect().await.unwrap();
    assert!(!context.tools.iter().any(|tool| tool.name == "record_write"));
}

struct Impostor(&'static str);

impl Capability for Impostor {
    fn id(&self) -> &str {
        self.0
    }

    fn name(&self) -> &str {
        "Impostor"
    }

    fn description(&self) -> &str {
        "Tries to replace a Framework capability."
    }
}

#[test]
fn host_capabilities_cannot_shadow_framework_or_each_other() {
    for id in ["session_file_system", "session_storage"] {
        let error = Agent::builder()
            .instructions("x")
            .model(Model::simulated("ok"))
            .host_services(HostServices::new().capability(Impostor(id)))
            .build()
            .unwrap_err();
        assert!(
            matches!(error, BuildError::DuplicateCapability { .. }),
            "{id}: {error:?}"
        );
    }

    let error = Agent::builder()
        .instructions("x")
        .model(Model::simulated("ok"))
        .capability("host_policy")
        .host_services(HostServices::new().capability(HostPolicy {
            effects: Effects::default(),
        }))
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::DuplicateCapability { .. }));

    let error = Agent::builder()
        .instructions("x")
        .model(Model::simulated("ok"))
        .host_services(
            HostServices::new()
                .capability(Impostor("twin"))
                .capability(Impostor("twin")),
        )
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::DuplicateCapability { .. }));

    let error = Agent::builder()
        .instructions("x")
        .model(Model::simulated("ok"))
        .tool(everruns::FunctionTool::new(
            "record_write",
            "duplicate",
            json!({"type": "object"}),
            |_args| async { Ok::<_, String>(json!({})) },
        ))
        .host_services(HostServices::new().capability(HostPolicy {
            effects: Effects::default(),
        }))
        .build()
        .unwrap_err();
    assert!(
        matches!(
            error,
            BuildError::DuplicateTool { .. } | BuildError::DuplicateCapability { .. }
        ),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// Host provider resolution.
// ---------------------------------------------------------------------------

const ACME: &str = "acme_driver";

/// Replies with what the driver was constructed from, so a test can see
/// which credentials and metadata reached the call.
struct EchoConfig {
    key: Option<String>,
    account: Option<String>,
}

#[async_trait]
impl ChatDriver for EchoConfig {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let reply = format!(
            "key={} account={} model={}",
            self.key.as_deref().unwrap_or("-"),
            self.account.as_deref().unwrap_or("-"),
            config.model
        );
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta(reply)),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

fn acme_drivers(created: Arc<AtomicUsize>) -> DriverRegistry {
    let mut drivers = DriverRegistry::new();
    drivers.register_external(ACME, move |config: &DriverConfig| {
        created.fetch_add(1, Ordering::SeqCst);
        Box::new(EchoConfig {
            key: config.api_key.clone(),
            account: config.metadata.account_id.clone(),
        })
    });
    drivers
}

#[derive(Default)]
struct VaultStore {
    key: Mutex<String>,
    sessions: Mutex<Vec<String>>,
}

#[async_trait]
impl ProviderStore for VaultStore {
    async fn get_model_spec(
        &self,
        _model_id: HostModelId,
    ) -> Result<Option<HostModelSpec>, AgentLoopError> {
        Ok(None)
    }

    async fn get_default_model_spec(&self) -> Result<Option<HostModelSpec>, AgentLoopError> {
        Ok(None)
    }

    async fn get_provider_config_for_session(
        &self,
        provider: &HostProviderKey,
        session: everruns::SessionId,
    ) -> Result<Option<ProviderConfig>, AgentLoopError> {
        self.sessions.lock().unwrap().push(session.to_string());
        self.get_provider_config(provider).await
    }

    async fn get_provider_config(
        &self,
        provider: &HostProviderKey,
    ) -> Result<Option<ProviderConfig>, AgentLoopError> {
        if provider.as_str() != "vault" {
            return Ok(None);
        }
        Ok(Some(
            ProviderConfig::for_provider(provider.clone(), DriverId::external(ACME))
                .with_api_key(self.key.lock().unwrap().clone())
                .with_metadata(ProviderMetadata {
                    account_id: Some("acct-7".into()),
                    ..ProviderMetadata::default()
                }),
        ))
    }
}

#[tokio::test]
async fn host_providers_resolve_credentials_per_call_and_preserve_metadata() {
    let store = Arc::new(VaultStore::default());
    *store.key.lock().unwrap() = "key-1".into();
    let created = Arc::new(AtomicUsize::new(0));
    let providers = HostProviders::shared(store.clone(), acme_drivers(created.clone()));
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::on("vault", "acme-large"))
        .host_services(HostServices::new().providers(providers))
        .build()
        .unwrap();
    let session = Engine::new().create(agent);

    let context = session.inspect().await.unwrap();
    assert_eq!(context.model.provider.as_str(), "vault");
    assert_eq!(context.model.model, "acme-large");

    let first = session.run("one").await.unwrap();
    assert_eq!(first.response, "key=key-1 account=acct-7 model=acme-large");

    // Rotation is live: the next call resolves again, without a new runtime.
    *store.key.lock().unwrap() = "key-2".into();
    let second = session.run("two").await.unwrap();
    assert_eq!(second.response, "key=key-2 account=acct-7 model=acme-large");

    let sessions = store.sessions.lock().unwrap().clone();
    assert!(!sessions.is_empty());
    assert!(
        sessions.iter().all(|id| *id == session.id()),
        "credentials resolve under the calling session's identity"
    );
    assert!(
        created.load(Ordering::SeqCst) >= 2,
        "each call constructs its driver from freshly resolved config"
    );
}

#[tokio::test]
async fn unknown_host_provider_fails_before_execution_without_fallback() {
    let created = Arc::new(AtomicUsize::new(0));
    // A driver registered under the model's provider name must not be guessed.
    let mut drivers = acme_drivers(created.clone());
    let fallback = created.clone();
    drivers.register_external("ghost", move |config: &DriverConfig| {
        fallback.fetch_add(100, Ordering::SeqCst);
        Box::new(EchoConfig {
            key: config.api_key.clone(),
            account: None,
        })
    });
    let providers = HostProviders::new(VaultStore::default(), drivers);
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::on("ghost", "acme-large"))
        .host_services(HostServices::new().providers(providers))
        .build()
        .unwrap();

    // Runtime construction refuses the unknown key; Engine suspends the
    // session instead of running it with a guessed provider.
    let outcome = Engine::new().create(agent).run("hello").await;
    assert!(
        outcome.as_ref().map_or(true, |turn| !turn.success),
        "turn must not succeed: {outcome:?}"
    );
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "no driver was constructed"
    );
}

struct Silent;

#[async_trait]
impl ChatDriver for Silent {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        Ok(Box::pin(futures::stream::pending()))
    }
}

#[test]
fn conflicting_model_and_provider_declarations_fail_to_build() {
    let providers = || HostProviders::new(VaultStore::default(), DriverRegistry::new());

    let error = Agent::builder()
        .instructions("x")
        .model(Model::on("vault", "m"))
        .provider(Provider::new("vault", Silent))
        .host_services(HostServices::new().providers(providers()))
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::ConflictingProviders { .. }));

    let error = Agent::builder()
        .instructions("x")
        .model(Model::new("m", Provider::new("bundled", Silent)))
        .host_services(HostServices::new().providers(providers()))
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::ConflictingProviders { .. }));

    let error = Agent::builder()
        .instructions("x")
        .model("plain-id")
        .host_services(HostServices::new().providers(providers()))
        .build()
        .unwrap_err();
    assert_eq!(error, BuildError::MissingProvider);

    let error = Agent::builder()
        .instructions("x")
        .model(Model::on("vault", "m"))
        .provider(Provider::new("other", Silent))
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::ConflictingProviders { .. }));

    Agent::builder()
        .instructions("x")
        .model(Model::on("same", "m"))
        .provider(Provider::new("same", Silent))
        .build()
        .expect("a model and provider naming the same key agree");
}

// ---------------------------------------------------------------------------
// Provider recovery budgets.
// ---------------------------------------------------------------------------

/// Fails with a transient provider error `failures` times, then answers.
struct Flaky {
    calls: Arc<AtomicUsize>,
    failures: usize,
}

#[async_trait]
impl ChatDriver for Flaky {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            return Err(AgentLoopError::llm_http(503, "", "temporarily unavailable"));
        }
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("recovered".into())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

fn retry(max_retries: u32) -> LlmRetryConfig {
    LlmRetryConfig {
        max_retries,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
        jitter_factor: 0.0,
        ..LlmRetryConfig::default()
    }
}

#[tokio::test]
async fn host_retry_budget_survives_engine_runtime_construction() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::new(
            "m",
            Provider::new(
                "flaky",
                Flaky {
                    calls: calls.clone(),
                    failures: 2,
                },
            ),
        ))
        .host_services(HostServices::new().provider_retry(retry(3)))
        .build()
        .unwrap();
    let turn = Engine::new().create(agent).run("hi").await.unwrap();
    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "recovered");
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::new(
            "m",
            Provider::new(
                "flaky",
                Flaky {
                    calls: calls.clone(),
                    failures: 2,
                },
            ),
        ))
        .host_services(HostServices::new().provider_retry(retry(0)))
        .build()
        .unwrap();
    let outcome = Engine::new().create(agent).run("hi").await;
    assert!(outcome.map_or(true, |turn| !turn.success));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry budget, one call");
}

#[tokio::test]
async fn host_stall_budget_survives_engine_runtime_construction() {
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::new("m", Provider::new("silent", Silent)))
        .host_services(
            HostServices::new()
                .provider_retry(retry(0))
                .provider_stall_timeout(Duration::from_millis(100)),
        )
        .build()
        .unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        Engine::new().create(agent).run("hi"),
    )
    .await
    .expect("the host stall budget ends a silent stream, not the 120s default");
    assert!(outcome.map_or(true, |turn| !turn.success));
}

// ---------------------------------------------------------------------------
// Host observers.
// ---------------------------------------------------------------------------

/// Event type and canonical sequence, as each observer saw them.
type Seen = Arc<Mutex<Vec<(String, Option<i32>)>>>;

#[derive(Clone, Default)]
struct AppRecorder(Seen);

#[async_trait]
impl EventListener for AppRecorder {
    async fn on_event(&self, event: &SessionEvent) {
        self.0
            .lock()
            .unwrap()
            .push((event.event_type().to_string(), event.sequence()));
    }
}

#[derive(Clone, Default)]
struct HostRecorder(Seen);

#[async_trait]
impl HostEventListener for HostRecorder {
    async fn on_event(&self, event: &HostEvent) {
        self.0
            .lock()
            .unwrap()
            .push((event.event_type.clone(), event.sequence));
    }
}

/// Never finishes handling an event.
struct Stuck;

#[async_trait]
impl HostEventListener for Stuck {
    async fn on_event(&self, _event: &HostEvent) {
        futures::future::pending::<()>().await;
    }

    fn name(&self) -> &'static str {
        "stuck"
    }
}

/// Panics on every event.
struct Failing;

#[async_trait]
impl HostEventListener for Failing {
    async fn on_event(&self, _event: &HostEvent) {
        panic!("host observer failure");
    }

    fn name(&self) -> &'static str {
        "failing"
    }
}

fn committed(events: &[(String, Option<i32>)]) -> BTreeSet<(String, i32)> {
    events
        .iter()
        .filter_map(|(kind, sequence)| sequence.map(|sequence| (kind.clone(), sequence)))
        .collect()
}

#[tokio::test]
async fn host_and_engine_observers_receive_the_same_committed_envelopes() {
    let app = AppRecorder::default();
    let host = HostRecorder::default();
    let engine = Engine::builder()
        .listener(app.clone())
        .host_listener(host.clone())
        .host_listener(Stuck)
        .host_listener(Failing)
        .build();
    let agent = Agent::builder()
        .instructions("Reply.")
        .model(Model::simulated("observed"))
        .build()
        .unwrap();
    let session = engine.create(agent);
    let turn = session.run("hi").await.unwrap();
    assert!(turn.success, "failing observers cannot fail the turn");
    let page = session.event_page(None, 1000).await.unwrap();
    let report = engine.shutdown(Duration::from_millis(500)).await;
    assert!(report.timed_out, "the stuck observer misses the deadline");

    let app = committed(&app.0.lock().unwrap());
    let host = committed(&host.0.lock().unwrap());
    assert!(!host.is_empty());
    assert_eq!(app, host, "both observer kinds see the same envelopes");
    let history: BTreeSet<_> = page
        .events
        .iter()
        .filter_map(|event| {
            event
                .sequence()
                .map(|sequence| (event.event_type().to_string(), sequence))
        })
        .collect();
    assert_eq!(
        history, host,
        "observation adds nothing to canonical history"
    );
}

// ---------------------------------------------------------------------------
// Call-time MCP credentials.
// ---------------------------------------------------------------------------

#[cfg(feature = "mcp-stdio")]
#[tokio::test]
async fn host_mcp_auth_is_asked_at_every_call_after_engine_construction() {
    use everruns::McpServer;
    use everruns::hosting::services::{McpAuthRequest, McpAuthResult, McpCredential};

    /// Mints a fresh token per request, like a refreshing credential source.
    #[derive(Clone, Default)]
    struct Refreshing(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl everruns::hosting::services::McpAuthProvider for Refreshing {
        async fn authorization(
            &self,
            request: &McpAuthRequest<'_>,
        ) -> McpAuthResult<Option<McpCredential>> {
            let mut seen = self.0.lock().unwrap();
            seen.push(request.server_name.to_string());
            Ok(Some(McpCredential::bearer(format!("token-{}", seen.len()))))
        }
    }

    let auth = Refreshing::default();
    let server =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_stdio_server.py");
    let (model, _captured) = scripted(vec![
        vec![call("mcp", "mcp_echo__echo", json!({"message": "hi"}))],
        vec![],
    ]);
    let agent = Agent::builder()
        .instructions("Use the echo MCP server.")
        .model(model)
        .mcp_server(McpServer::stdio("echo", "python3").arg(server.to_string_lossy()))
        .host_services(HostServices::new().mcp_auth(auth.clone()))
        .build()
        .unwrap();
    let session = Engine::new().create(agent);

    let context = session.inspect().await.unwrap();
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "mcp_echo__echo")
    );
    let after_discovery = auth.0.lock().unwrap().len();
    assert!(
        after_discovery >= 1,
        "discovery asked the host for credentials"
    );

    let turn = session.run("echo hi").await.unwrap();
    assert!(turn.success, "{:?}", turn.error);
    let requests = auth.0.lock().unwrap().clone();
    assert!(
        requests.len() > after_discovery,
        "the tool call asked again rather than reusing a construction-time token"
    );
    assert!(requests.iter().all(|server| server == "echo"));
}
