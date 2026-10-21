//! Production runtime lifecycle checks for tools added, redefined, and revoked
//! while a session remains alive. The recording driver observes the actual
//! provider-neutral request config assembled by the runtime.

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::error::Result;
use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
use everruns_contracts::runtime::tool_surface::{
    NativeToolDisclosure, NativeToolDisclosurePolicy, ToolSurface, ToolSurfaceMode,
    ToolSurfacePlan, ToolSurfaceRoute,
};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};
use everruns_core::host::{AgentBuilder, HarnessBuilder, HostComposition, SessionBuilder};
use everruns_core::{Capability, CapabilityRegistry, CapabilityStatus, Tool, ToolExecutionResult};
use everruns_llmsim::LlmSimRuntimeExt;
use futures::stream;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

const MICROSOFT_DOCS_PLUGIN_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../core/testdata/plugins/microsoft-docs"
);

#[derive(Clone)]
enum Reply {
    Text(&'static str),
    Call(&'static str, Value),
}

#[derive(Clone, Debug)]
struct RequestSnapshot {
    current_tools: Value,
    baseline_tools: Option<Value>,
    system_prompt: Vec<String>,
    baseline_prompt: Option<String>,
    changes: Vec<everruns_contracts::tool_changes::ToolChanges>,
}

#[derive(Default)]
struct Recorder {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RequestSnapshot>>,
}

impl Recorder {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

struct RecordingDriver(Arc<Recorder>);

#[async_trait]
impl ChatDriver for RecordingDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_contracts::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.0.requests.lock().unwrap().push(RequestSnapshot {
            current_tools: serde_json::to_value(&config.tools).unwrap(),
            baseline_tools: config
                .tool_baseline
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .unwrap(),
            system_prompt: messages
                .iter()
                .filter(|message| message.role == everruns_contracts::message::MessageRole::System)
                .map(Message::content_as_text)
                .collect(),
            baseline_prompt: config.system_prompt_baseline.clone(),
            changes: messages
                .into_iter()
                .filter_map(|message| message.tool_changes)
                .collect(),
        });

        let reply = self
            .0
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test supplied a response for every model request");
        let events = match reply {
            Reply::Text(text) => vec![
                Ok(LlmStreamEvent::TextDelta(text.to_string())),
                Ok(LlmStreamEvent::Done(Box::new(
                    LlmCompletionMetadata::default(),
                ))),
            ],
            Reply::Call(name, arguments) => vec![
                Ok(LlmStreamEvent::ToolCalls(vec![ToolCall {
                    id: "recorded-call".into(),
                    name: name.into(),
                    arguments,
                }])),
                Ok(LlmStreamEvent::Done(Box::new(
                    LlmCompletionMetadata::default(),
                ))),
            ],
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[derive(Default)]
struct PluginState {
    schema_version: Mutex<u8>,
    executed_versions: Mutex<Vec<u8>>,
}

struct DynamicPlugin(Arc<PluginState>);

impl Capability for DynamicPlugin {
    fn id(&self) -> &str {
        "dynamic_plugin"
    }

    fn name(&self) -> &str {
        "Dynamic Plugin"
    }

    fn description(&self) -> &str {
        "A plugin capability used to verify live tool catalog changes."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("DYNAMIC_PLUGIN_PROMPT")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(DynamicTool(self.0.clone()))]
    }
}

struct DynamicTool(Arc<PluginState>);

#[async_trait]
impl Tool for DynamicTool {
    fn name(&self) -> &str {
        "dynamic_plugin_action"
    }

    fn description(&self) -> &str {
        "Record which live schema version executed."
    }

    fn parameters_schema(&self) -> Value {
        match *self.0.schema_version.lock().unwrap() {
            0 => json!({
                "type": "object",
                "properties": { "label": { "type": "string" } },
                "required": ["label"],
                "additionalProperties": false
            }),
            _ => json!({
                "type": "object",
                "properties": { "count": { "type": "integer" } },
                "required": ["count"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        let version = *self.0.schema_version.lock().unwrap();
        self.0.executed_versions.lock().unwrap().push(version);
        ToolExecutionResult::success(json!({ "schema_version": version }))
    }
}

#[test]
fn hybrid_surface_keeps_cli_routes_and_defers_eligible_native_schemas() {
    let builtin = |name: &str, deferrable| {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: "Test operation.".into(),
            parameters: json!({"type":"object"}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    };
    let definitions = vec![
        builtin("shared_operation", DeferrablePolicy::Automatic),
        builtin("hot_operation", DeferrablePolicy::Never),
        ToolDefinition::function("native_only", "Native only.", json!({"type":"object"})),
    ];
    let mut plan = ToolSurfacePlan::for_mode(ToolSurfaceMode::Hybrid);
    plan.set_native_disclosure_policy(NativeToolDisclosurePolicy {
        default: NativeToolDisclosure::Deferred,
        overrides: Default::default(),
    });
    plan.extend_definitions(&definitions, |definition| {
        definition.name() != "native_only"
    });

    assert_eq!(
        plan.route_for("shared_operation"),
        Some(ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Deferred,
        })
    );
    assert_eq!(
        plan.route_for("hot_operation"),
        Some(ToolSurfaceRoute {
            surface: ToolSurface::Both,
            native_disclosure: NativeToolDisclosure::Eager,
        })
    );
    assert_eq!(
        plan.route_for("native_only"),
        Some(ToolSurfaceRoute {
            surface: ToolSurface::Native,
            native_disclosure: NativeToolDisclosure::Eager,
        })
    );
    assert_eq!(
        plan.route_for("unknown"),
        None,
        "unknown routes fail closed"
    );
}

#[test]
fn large_catalog_search_is_bounded_and_obeys_current_authorization() {
    let definitions: Vec<_> = (0..2_000)
        .map(|index| {
            ToolDefinition::Builtin(BuiltinTool {
                name: format!("mcp_docs__search_{index:04}"),
                display_name: None,
                description: format!("Search document collection {index} by identifier."),
                parameters: json!({
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                }),
                policy: ToolPolicy::Auto,
                category: Some("docs".into()),
                deferrable: DeferrablePolicy::Automatic,
                hints: ToolHints::default().with_metadata(json!({
                    "mcp_server": {
                        "name": "docs",
                        "description": "Internal product documentation and release notes"
                    }
                })),
                full_parameters: None,
            })
        })
        .collect();
    let index = ToolSearchIndex::new(&definitions);

    let exact = index.search("mcp_docs__search_1999", 4, |_| true);
    assert!(exact.len() <= 4, "the requested hit limit is enforced");
    assert!(
        exact
            .iter()
            .any(|hit| { hit.name == "mcp_docs__search_1999" && hit.tool_match })
    );

    let server = index.search("release notes", 7, |_| true);
    assert_eq!(server.len(), 7, "the caller's result limit is enforced");
    assert!(server.iter().all(|hit| hit.server_match));
    assert!(server.iter().all(|hit| hit.name.starts_with("mcp_docs__")));

    let revoked = index.search("mcp_docs__search_1999", 4, |name| {
        name != "mcp_docs__search_1999"
    });
    assert!(
        revoked
            .iter()
            .all(|hit| hit.name != "mcp_docs__search_1999"),
        "revoked tool cannot appear in search results, though other similar tools can"
    );
}

#[tokio::test]
async fn runtime_records_plugin_addition_redefinition_and_revocation() {
    let recorder = Arc::new(Recorder::new([
        Reply::Text("initial baseline"),
        Reply::Call("dynamic_plugin_action", json!({ "label": "v1" })),
        Reply::Text("plugin call completed"),
        Reply::Call("dynamic_plugin_action", json!({ "count": 2 })),
        Reply::Text("redefined plugin call completed"),
        Reply::Call("dynamic_plugin_action", json!({ "count": 3 })),
        Reply::Text("removed plugin call was refused"),
    ]));
    let platform = HostComposition::builder()
        .capability_registry(CapabilityRegistry::new())
        .build();
    let harness_id = everruns_contracts::typed_id::HarnessId::from_seed(3401);
    let agent_id = everruns_contracts::typed_id::AgentId::from_seed(3401);
    let session_id = everruns_contracts::typed_id::SessionId::from_seed(3401);
    let provider = everruns_contracts::runtime_provider::Provider::new(
        "recording",
        RecordingDriver(recorder.clone()),
    );

    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform)
        .provider_with_default_model(provider, "recording-model")
        .harness(
            HarnessBuilder::new("catalog", "Stable harness instructions.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("catalog-agent", "Stable agent instructions.")
                .id(agent_id)
                .max_iterations(5)
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
        .expect("runtime builds");

    runtime
        .run_text_turn(session_id, "Establish the stable baseline")
        .await
        .expect("baseline turn runs");

    let plugin_state = Arc::new(PluginState::default());
    runtime
        .register_capability(Arc::new(DynamicPlugin(plugin_state.clone())))
        .expect("dynamic plugin registers on the running runtime");
    let activated = runtime
        .activate_capability(session_id, "dynamic_plugin")
        .await
        .expect("plugin activates");
    assert!(activated.changed && activated.active);
    let added = runtime
        .load_context(session_id)
        .await
        .expect("context includes the plugin")
        .runtime_agent;
    let added_tool = added
        .tools
        .iter()
        .find(|tool| tool.name() == "dynamic_plugin_action")
        .expect("plugin tool appears after activation");
    assert_eq!(
        added_tool.parameters().clone(),
        json!({
            "type": "object",
            "properties": { "label": { "type": "string" } },
            "required": ["label"],
            "additionalProperties": false
        })
    );
    runtime
        .run_text_turn(session_id, "Use the newly attached plugin")
        .await
        .expect("plugin addition turn runs");

    *plugin_state.schema_version.lock().unwrap() = 1;
    let redefined = runtime
        .load_context(session_id)
        .await
        .expect("context reloads the plugin schema")
        .runtime_agent;
    let redefined_tool = redefined
        .tools
        .iter()
        .find(|tool| tool.name() == "dynamic_plugin_action")
        .expect("tool name remains present");
    assert_eq!(
        redefined_tool.parameters().clone(),
        json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } },
            "required": ["count"],
            "additionalProperties": false
        })
    );
    runtime
        .run_text_turn(session_id, "Use the redefined plugin")
        .await
        .expect("plugin redefinition turn runs");

    let removed = runtime
        .deactivate_capability(session_id, "dynamic_plugin")
        .await
        .expect("plugin deactivates");
    assert!(removed.changed && !removed.active);
    runtime
        .run_text_turn(session_id, "Try the removed plugin")
        .await
        .expect("removed call is handled as a tool failure");

    let requests = recorder.requests.lock().unwrap();
    assert_eq!(requests.len(), 7, "each model pass is captured");
    let first = &requests[0];
    assert!(
        first
            .system_prompt
            .join("\n")
            .contains("Stable harness instructions.")
    );
    assert!(
        first
            .system_prompt
            .join("\n")
            .contains("Stable agent instructions.")
    );
    let baseline = first
        .baseline_tools
        .as_ref()
        .expect("first provider request records its baseline");
    assert!(!baseline.to_string().contains("dynamic_plugin_action"));

    let add_request = &requests[1];
    assert_eq!(add_request.baseline_tools.as_ref(), Some(baseline));
    assert_eq!(
        add_request.baseline_prompt.as_deref(),
        Some(first.system_prompt.join("\n\n").as_str())
    );
    assert!(
        add_request.changes.iter().any(|change| {
            change
                .added
                .iter()
                .any(|tool| tool.name() == "dynamic_plugin_action")
        }),
        "plugin attachment is recorded as an addition"
    );
    assert!(
        add_request.changes.iter().any(|change| change
            .note
            .as_deref()
            .is_some_and(|note| note.contains("DYNAMIC_PLUGIN_PROMPT"))),
        "plugin prompt text is carried as an incremental note"
    );

    let redefine_request = &requests[3];
    assert_eq!(redefine_request.baseline_tools.as_ref(), Some(baseline));
    assert!(
        redefine_request.changes.iter().any(|change| {
            change
                .redefined
                .iter()
                .any(|tool| tool.name() == "dynamic_plugin_action")
        }),
        "a same-name schema change is recorded as a redefinition"
    );
    assert!(
        redefine_request
            .current_tools
            .to_string()
            .contains("\"count\"")
    );

    let removal_request = &requests[5];
    assert!(
        removal_request
            .current_tools
            .to_string()
            .contains("dynamic_plugin_action"),
        "a removal on a non-inline provider stays held in the warm baseline"
    );
    assert!(
        removal_request.changes.iter().all(|change| {
            !change
                .removed
                .iter()
                .any(|name| name == "dynamic_plugin_action")
        }),
        "held removals are not advertised as removed until a cache rewrite is needed"
    );
    assert_eq!(
        *plugin_state.executed_versions.lock().unwrap(),
        vec![0, 1],
        "the removed definition must not execute"
    );
}

#[tokio::test]
async fn plugin_namespace_definition_contributes_to_runtime_context() {
    let builder = everruns::batteries::runtime_builder()
        .llm_sim_as_default(everruns_llmsim::LlmSimConfig::fixed("ok"))
        .with_plugin_dir(Path::new(MICROSOFT_DOCS_PLUGIN_DIR))
        .expect("the declarative plugin fixture compiles");
    let mut plugin_capability = builder
        .plugin_capability("microsoft-docs")
        .expect("plugin registry exposes the plugin capability");
    // Keep this contribution test offline: the fixture's MCP URL targets a
    // real service, while the separate MCP lifecycle test uses a fake egress.
    let mut plugin_config = plugin_capability.config_value().clone();
    plugin_config["mcp_servers"] = Value::Null;
    plugin_capability.set_config(plugin_config);
    let runtime = builder
        .single_session(|session| {
            session
                .harness("plugin catalog", "Plugin lifecycle contract test.")
                .agent("plugin catalog agent", "Use plugin docs when needed.")
                .agent_capability(plugin_capability)
        })
        .build()
        .await
        .expect("runtime builds with a plugin: capability");
    let session_id = runtime.default_session_id().expect("single session exists");

    let context = runtime
        .load_context(session_id)
        .await
        .expect("plugin context loads");
    assert!(
        context
            .runtime_agent
            .system_prompt
            .contains("docs-researcher"),
        "the plugin: declarative definition contributes its agent prompt"
    );
    assert!(
        context
            .resolved_capability_configs
            .iter()
            .any(|config| config.capability_id() == "plugin:microsoft-docs"),
        "runtime resolves the plugin namespace as an installed capability"
    );
}
