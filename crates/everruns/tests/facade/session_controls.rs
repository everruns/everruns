//! Session commands, live capability controls and host queries (EVE-1297).
//! Imports only the public `everruns` API.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use everruns::hosting::services::{
    Capability, CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult,
    CommandSource, ExecuteCommandRequest, Result as HostResult, Tool, ToolCall,
    ToolExecutionResult, async_trait,
};
use everruns::hosting::spi::HostBackends;
use everruns::llm::Message;
use everruns::{
    Agent, CapabilityRef, CapabilityRegistrationError, Engine, FunctionTool, LlmSimConfig,
    McpServer, Model, ReasoningEffort, Session, SessionControlError, SessionId,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

const EXT_MARKER: &str = "HOT_EXTENSION_INSTRUCTIONS";
const DEADLINE: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// A hot-installable extension: a tool, instructions and a slash command whose
// result reports the session context the command saw.
// ---------------------------------------------------------------------------

struct ExtTool;

#[async_trait]
impl Tool for ExtTool {
    fn name(&self) -> &str {
        "ext_tool"
    }

    fn description(&self) -> &str {
        "Extension tool."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({ "ext": true }))
    }
}

#[derive(Default)]
struct Extension {
    runs: Arc<AtomicUsize>,
    /// When set, `/ext wait` blocks until notified.
    gate: Option<Arc<Notify>>,
}

#[async_trait]
impl Capability for Extension {
    fn id(&self) -> &str {
        "hot_extension"
    }

    fn name(&self) -> &str {
        "Hot extension"
    }

    fn description(&self) -> &str {
        "Installed after startup."
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(EXT_MARKER)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ExtTool)]
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "ext".into(),
            description: "Report the extension's view.".into(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "mode".into(),
                description: "What to report.".into(),
                required: false,
                suggestions: vec!["model".into(), "wait".into()],
            }],
        }]
    }

    async fn execute_command(
        &self,
        request: &ExecuteCommandRequest,
        context: &CommandExecutionContext,
    ) -> HostResult<CommandResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if request.arguments.as_deref() == Some("wait")
            && let Some(gate) = &self.gate
        {
            gate.notified().await;
        }
        let turn = context.host.turn_context().await?;
        Ok(CommandResult {
            success: true,
            message: format!(
                "ext model={} instructions_have_marker={}",
                turn.model,
                turn.system_prompt.contains(EXT_MARKER)
            ),
            error_code: None,
            error_fields: None,
        })
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

fn scripted(calls: Vec<Vec<ToolCall>>) -> (LlmSimConfig, Captured) {
    let captured: Captured = Arc::default();
    let mut config = LlmSimConfig::fixed("done").with_tool_call_sequence(calls);
    config.message_capture = Some(captured.clone());
    (config, captured)
}

fn text_of(call: &[Message]) -> String {
    call.iter()
        .map(|message| message.content.to_text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn last_call(captured: &Captured) -> String {
    text_of(captured.lock().unwrap().last().expect("a model call"))
}

/// A session handle a tool of the session's own turn can reach.
#[derive(Clone, Default)]
struct Slot(Arc<Mutex<Option<Session>>>);

impl Slot {
    fn get(&self) -> Session {
        self.0.lock().unwrap().clone().expect("session installed")
    }

    fn set(&self, session: &Session) {
        *self.0.lock().unwrap() = Some(session.clone());
    }

    /// Break the Session -> Agent -> tool -> Session cycle.
    fn clear(&self) {
        self.0.lock().unwrap().take();
    }
}

/// The model-facing `run_command` tool a host routes through the session's
/// command registry.
fn run_command_tool(slot: Slot) -> FunctionTool {
    FunctionTool::new(
        "run_command",
        "Run a slash command.",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "arguments": { "type": "string" }
            },
            "required": ["name"]
        }),
        move |arguments: Value| {
            let session = slot.get();
            async move {
                let output = session
                    .run_command(
                        arguments["name"].as_str().unwrap_or_default(),
                        arguments["arguments"].as_str(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(json!({ "message": output.message }))
            }
        },
    )
}

/// A tool that activates or deactivates the extension from inside a turn.
fn toggle_tool(slot: Slot) -> FunctionTool {
    FunctionTool::new(
        "toggle_extension",
        "Turn the extension on or off.",
        json!({
            "type": "object",
            "properties": { "on": { "type": "boolean" } },
            "required": ["on"]
        }),
        move |arguments: Value| {
            let session = slot.get();
            async move {
                let change = if arguments["on"].as_bool().unwrap_or(false) {
                    session
                        .activate_capability(CapabilityRef::new("hot_extension"))
                        .await
                } else {
                    session.deactivate_capability("hot_extension").await
                }
                .map_err(|error| error.to_string())?;
                Ok::<_, String>(json!({ "active": change.active }))
            }
        },
    )
}

/// An Agent on the explicit legacy lane (caller-owned backends), returning
/// the backends so tests can read the persisted session record.
fn legacy_agent(
    model: Model,
    tools: Vec<FunctionTool>,
    services: Option<everruns::hosting::services::HostServices>,
) -> (Agent, HostBackends) {
    let backends = HostBackends::in_memory();
    let mut builder = Agent::builder()
        .instructions("Use tools when asked.")
        .model(model)
        .backends(backends.clone());
    for tool in tools {
        builder = builder.tool(tool);
    }
    if let Some(services) = services {
        builder = builder.host_services(services);
    }
    (builder.build().expect("valid agent"), backends)
}

async fn event_count(session: &Session) -> usize {
    session.events_after(0).await.expect("events").len()
}

async fn has_tool(session: &Session, name: &str) -> bool {
    session
        .inspect()
        .await
        .expect("context")
        .tools
        .iter()
        .any(|tool| tool.name == name)
}

// ---------------------------------------------------------------------------
// Command discovery and dispatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_handle_discovers_the_same_commands_and_dispatch_matches() {
    let services =
        everruns::hosting::services::HostServices::new().capability(Extension::default());
    let (agent, _) = legacy_agent(Model::simulated("ok"), vec![], Some(services));
    let engine = Engine::new();
    let session = engine.create(agent);

    let listed = session.commands().await.expect("commands");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "ext");
    assert_eq!(listed[0].arguments[0].suggestions, ["model", "wait"]);
    assert_eq!(session.inspect().await.unwrap().commands, listed);
    assert_eq!(session.command_context().await.unwrap().commands, listed);
    let resumed = engine.resume(session.session_id()).await.expect("resume");
    assert_eq!(resumed.commands().await.unwrap(), listed);

    let before = event_count(&session).await;
    let output = session
        .run_command("/ext", Some("model"))
        .await
        .expect("run");
    assert!(output.success);
    assert_eq!(
        output.message,
        "ext model=llmsim-model instructions_have_marker=true"
    );
    assert_eq!(resumed.run_command("ext", None).await.unwrap(), output);
    assert!(matches!(
        session.run_command("missing", None).await,
        Err(SessionControlError::Rejected(_))
    ));
    assert_eq!(
        event_count(&session).await,
        before,
        "commands and queries never append to canonical history"
    );
    assert!(session.history().page().await.unwrap().messages.is_empty());
}

#[tokio::test]
async fn a_run_command_tool_dispatches_during_the_active_turn() {
    let slot = Slot::default();
    let (config, captured) = scripted(vec![
        vec![call(
            "c1",
            "run_command",
            json!({"name": "/ext", "arguments": "model"}),
        )],
        vec![],
    ]);
    let services =
        everruns::hosting::services::HostServices::new().capability(Extension::default());
    let (agent, _) = legacy_agent(
        Model::simulated_with_config(config),
        vec![run_command_tool(slot.clone())],
        Some(services),
    );
    let session = Engine::new().create(agent);
    slot.set(&session);

    let turn = tokio::time::timeout(DEADLINE, session.run("run /ext"))
        .await
        .expect("no deadlock on the session's own mailbox")
        .expect("turn");
    assert!(turn.success, "{:?}", turn.error);
    assert!(
        last_call(&captured).contains("ext model=llmsim-model"),
        "the command result reached the model within the same turn"
    );
    let history = session.history().page().await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.role == everruns::MessageRole::User)
            .count(),
        1,
        "no nested agent turn"
    );
    slot.clear();
}

// ---------------------------------------------------------------------------
// Trusted registration separate from activation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_hot_installed_extension_registers_activates_runs_and_deactivates() {
    let (config, captured) = scripted(vec![]);
    let (agent, backends) = legacy_agent(Model::simulated_with_config(config), vec![], None);
    let engine = Engine::new();
    let session = engine.create(agent);
    assert!(session.run("before install").await.unwrap().success);

    let extension: Arc<dyn Capability> = Arc::new(Extension::default());
    engine
        .register_shared_capability(extension.clone())
        .expect("register");
    engine
        .register_shared_capability(extension)
        .expect("registering the same value again is a no-op");
    assert_eq!(
        engine.register_capability(Extension::default()),
        Err(CapabilityRegistrationError::Conflict {
            id: "hot_extension".into()
        })
    );

    // Registration alone changes no model surface.
    assert!(!has_tool(&session, "ext_tool").await);
    assert!(session.commands().await.unwrap().is_empty());

    let change = session
        .activate_capability(CapabilityRef::new("hot_extension"))
        .await
        .expect("activate");
    assert!(change.changed && change.active);
    let again = session
        .activate_capability(CapabilityRef::new("hot_extension"))
        .await
        .unwrap();
    assert!(!again.changed && again.active);
    let record = backends
        .session_store
        .get_session(session.session_id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        record
            .capabilities
            .iter()
            .any(|capability| capability.capability_id() == "hot_extension"),
        "activation persists as the session capability overlay"
    );

    assert!(has_tool(&session, "ext_tool").await);
    let output = session.run_command("ext", None).await.expect("run");
    assert!(output.message.contains("instructions_have_marker=true"));
    assert!(session.run("with extension").await.unwrap().success);
    assert!(last_call(&captured).contains(EXT_MARKER));

    let change = session
        .deactivate_capability("hot_extension")
        .await
        .expect("deactivate");
    assert!(change.changed && !change.active);
    assert!(!has_tool(&session, "ext_tool").await);
    assert!(session.commands().await.unwrap().is_empty());
    assert!(matches!(
        session.run_command("ext", None).await,
        Err(SessionControlError::Rejected(_))
    ));
    assert!(session.run("without extension").await.unwrap().success);
    assert!(
        !last_call(&captured).contains(EXT_MARKER),
        "the deactivated extension is absent from the next model surface"
    );
}

#[tokio::test]
async fn registration_rejects_framework_ids_and_activation_rejects_unknown_or_inherited() {
    struct Shadow;
    impl Capability for Shadow {
        fn id(&self) -> &str {
            "current_time"
        }
        fn name(&self) -> &str {
            "Shadow"
        }
        fn description(&self) -> &str {
            "Tries to replace a Framework capability."
        }
    }
    let engine = Engine::new();
    assert!(matches!(
        engine.register_capability(Shadow),
        Err(CapabilityRegistrationError::Conflict { .. })
    ));

    let backends = HostBackends::in_memory();
    let agent = Agent::builder()
        .instructions("x")
        .model(Model::simulated("ok"))
        .capability("current_time")
        .backends(backends.clone())
        .build()
        .unwrap();
    let session = engine.create(agent);
    assert!(matches!(
        session
            .activate_capability(CapabilityRef::new("not_registered"))
            .await,
        Err(SessionControlError::Rejected(_))
    ));
    assert!(
        matches!(
            session.deactivate_capability("current_time").await,
            Err(SessionControlError::Rejected(_))
        ),
        "an Agent capability is not removable at the session layer"
    );
    assert!(has_tool(&session, "get_current_time").await);
}

// ---------------------------------------------------------------------------
// Ordering against active sends, cancellation and teardown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_change_inside_a_turn_applies_from_its_next_model_call() {
    let slot = Slot::default();
    let (config, captured) = scripted(vec![
        vec![call("on", "toggle_extension", json!({"on": true}))],
        vec![],
        vec![call("off", "toggle_extension", json!({"on": false}))],
        vec![],
    ]);
    let (agent, _) = legacy_agent(
        Model::simulated_with_config(config),
        vec![toggle_tool(slot.clone())],
        None,
    );
    let engine = Engine::new();
    engine.register_capability(Extension::default()).unwrap();
    let session = engine.create(agent);
    slot.set(&session);

    let turn = tokio::time::timeout(DEADLINE, session.run("turn it on"))
        .await
        .expect("no deadlock")
        .unwrap();
    assert!(turn.success, "{:?}", turn.error);
    {
        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(!text_of(&calls[0]).contains(EXT_MARKER));
        assert!(
            text_of(&calls[1]).contains(EXT_MARKER),
            "the running turn's next model call sees the activation"
        );
    }
    let turn = tokio::time::timeout(DEADLINE, session.run("turn it off"))
        .await
        .expect("no deadlock")
        .unwrap();
    assert!(turn.success);
    {
        let calls = captured.lock().unwrap();
        assert!(text_of(&calls[2]).contains(EXT_MARKER));
        assert!(!text_of(&calls[3]).contains(EXT_MARKER));
    }
    slot.clear();
}

#[tokio::test]
async fn a_control_awaited_before_a_steering_send_reaches_that_send() {
    let (config, captured) = scripted(vec![]);
    let config = config.with_response_delay(Duration::from_millis(300));
    let (agent, _) = legacy_agent(Model::simulated_with_config(config), vec![], None);
    let engine = Engine::new();
    engine.register_capability(Extension::default()).unwrap();
    let session = engine.create(agent);

    let first = session.send("first").await.unwrap();
    session
        .activate_capability(CapabilityRef::new("hot_extension"))
        .await
        .expect("activation is answered while the turn runs");
    let steered = session.send("second").await.unwrap();
    assert!(first.wait().await.unwrap().success);
    assert!(steered.wait().await.unwrap().success);
    let calls = captured.lock().unwrap();
    let last = text_of(calls.last().unwrap());
    assert!(last.contains("second") && last.contains(EXT_MARKER));
}

#[tokio::test]
async fn cancellation_and_abandoned_commands_leave_the_session_usable() {
    let gate = Arc::new(Notify::new());
    let runs = Arc::new(AtomicUsize::new(0));
    let slot = Slot::default();
    let (config, _) = scripted(vec![
        vec![call(
            "w",
            "run_command",
            json!({"name": "ext", "arguments": "wait"}),
        )],
        vec![],
    ]);
    let services = everruns::hosting::services::HostServices::new().capability(Extension {
        runs: runs.clone(),
        gate: Some(gate.clone()),
    });
    let (agent, _) = legacy_agent(
        Model::simulated_with_config(config),
        vec![run_command_tool(slot.clone())],
        Some(services),
    );
    let engine = Engine::new();
    let session = engine.create(agent);
    slot.set(&session);

    // A turn blocked inside a command its own tool dispatched is cancellable.
    let sent = session.send("block").await.unwrap();
    tokio::time::timeout(DEADLINE, async {
        while runs.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("command started");
    tokio::time::timeout(DEADLINE, sent.turn().cancel())
        .await
        .expect("cancel is answered while a command runs")
        .expect("cancel");
    let turn = sent.wait().await.unwrap();
    assert_eq!(turn.stop_reason, everruns::TurnStopReason::Cancelled);

    // An abandoned host-side command does not wedge the mailbox.
    let pending = tokio::spawn({
        let session = session.clone();
        async move { session.run_command("ext", Some("wait")).await }
    });
    tokio::time::timeout(DEADLINE, async {
        while runs.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second command started");
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());

    let output = tokio::time::timeout(DEADLINE, session.run_command("ext", Some("model")))
        .await
        .expect("commands still answered")
        .unwrap();
    assert!(output.success);
    let users = session
        .history()
        .page()
        .await
        .unwrap()
        .messages
        .iter()
        .filter(|message| message.role == everruns::MessageRole::User)
        .count();
    assert_eq!(users, 1, "no command started a turn");

    // Host teardown: after shutdown the session answers promptly.
    engine.shutdown(Duration::from_secs(1)).await;
    tokio::time::timeout(DEADLINE, session.commands())
        .await
        .expect("no hang after shutdown")
        .unwrap();
    slot.clear();
    drop(engine);
    gate.notify_waiters();
}

// ---------------------------------------------------------------------------
// Live model, reasoning effort and MCP configuration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_model_and_effort_apply_to_the_next_call_and_unavailable_ones_change_nothing() {
    let efforts = Arc::new(Mutex::new(Vec::new()));
    let mut config = LlmSimConfig::fixed("done");
    config.effort_capture = Some(efforts.clone());
    let (agent, backends) = legacy_agent(Model::simulated_with_config(config), vec![], None);
    let session = Engine::new().create(agent);
    let id = session.session_id();
    let saved = || async {
        serde_json::to_value(backends.session_store.get_session(id).await.unwrap()).unwrap()
    };

    session
        .set_reasoning_effort(Some(ReasoningEffort::High))
        .await
        .expect("effort");
    let selected = session.set_model("llmsim-large").await.expect("model");
    assert_eq!(selected.model, "llmsim-large");
    assert_eq!(selected.provider.as_str(), "llmsim");
    assert_eq!(session.inspect().await.unwrap().model, selected);
    assert_eq!(session.command_context().await.unwrap().model, selected);
    assert!(session.run("hi").await.unwrap().success);
    assert_eq!(
        efforts.lock().unwrap().last().unwrap().as_deref(),
        Some("high")
    );

    let before = saved().await;
    for unavailable in [
        Model::on("missing_provider", "x"),
        Model::simulated("bundled providers cannot be attached live"),
        Model::from(" "),
    ] {
        assert!(matches!(
            session.set_model(unavailable).await,
            Err(SessionControlError::Rejected(_))
        ));
    }
    assert_eq!(
        session.inspect().await.unwrap().model,
        selected,
        "no fallback"
    );
    assert_eq!(saved().await, before, "no saved-state mutation");

    session.reset_model().await.unwrap();
    session.set_reasoning_effort(None).await.unwrap();
    assert_eq!(session.inspect().await.unwrap().model.model, "llmsim-model");
    assert!(session.run("again").await.unwrap().success);
    assert_eq!(efforts.lock().unwrap().last().unwrap(), &None);
}

#[tokio::test]
async fn an_effort_change_inside_a_turn_applies_to_its_next_model_call() {
    let slot = Slot::default();
    let efforts = Arc::new(Mutex::new(Vec::new()));
    let mut config = LlmSimConfig::fixed("done")
        .with_tool_call_sequence(vec![vec![call("e", "raise_effort", json!({}))], vec![]]);
    config.effort_capture = Some(efforts.clone());
    let raise = FunctionTool::new(
        "raise_effort",
        "Think harder.",
        json!({"type": "object"}),
        {
            let slot = slot.clone();
            move |_: Value| {
                let session = slot.get();
                async move {
                    session
                        .set_reasoning_effort(Some(ReasoningEffort::High))
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(json!({ "effort": "high" }))
                }
            }
        },
    );
    let (agent, _) = legacy_agent(Model::simulated_with_config(config), vec![raise], None);
    let session = Engine::new().create(agent);
    slot.set(&session);
    let turn = tokio::time::timeout(DEADLINE, session.run("think"))
        .await
        .expect("no deadlock")
        .unwrap();
    assert!(turn.success);
    assert_eq!(
        *efforts.lock().unwrap(),
        [None, Some("high".to_string())],
        "the selection reaches the running turn's next model call"
    );
    assert!(session.run("next turn").await.unwrap().success);
    assert_eq!(
        efforts.lock().unwrap().last().unwrap().as_deref(),
        Some("high"),
        "and every later turn starts from it"
    );
    slot.clear();
}

#[tokio::test]
async fn live_selections_survive_handle_drops_within_the_engine() {
    let (agent, _) = legacy_agent(Model::simulated("ok"), vec![], None);
    let engine = Engine::new();
    let session = engine.create(agent);
    let id: SessionId = session.session_id();
    session.set_model("llmsim-large").await.unwrap();
    session
        .add_mcp_server(McpServer::http("docs", "https://mcp.invalid/mcp").tool_discovery(false))
        .await
        .unwrap();
    drop(session);
    let resumed = engine.resume(id).await.unwrap();
    assert_eq!(resumed.inspect().await.unwrap().model.model, "llmsim-large");
    assert!(matches!(resumed.remove_mcp_server("docs").await, Ok(())));
}

#[tokio::test]
async fn live_mcp_servers_are_scoped_and_validated() {
    let (agent, backends) = legacy_agent(Model::simulated("ok"), vec![], None);
    let engine = Engine::new();
    let session = engine.create(agent);
    let id = session.session_id();
    let servers = || async {
        backends
            .session_store
            .get_session(id)
            .await
            .unwrap()
            .unwrap()
            .mcp_servers
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };

    session.inspect().await.unwrap();
    assert!(servers().await.is_empty());
    session
        .add_mcp_server(McpServer::http("docs", "https://mcp.invalid/mcp").tool_discovery(false))
        .await
        .expect("add");
    assert_eq!(servers().await, ["docs"]);
    session.refresh_mcp().await.expect("refresh");

    assert!(matches!(
        session.add_mcp_server(McpServer::http("blank", " ")).await,
        Err(SessionControlError::Rejected(_))
    ));
    assert!(matches!(
        session.remove_mcp_server("unknown").await,
        Err(SessionControlError::Rejected(_))
    ));
    assert_eq!(servers().await, ["docs"], "rejected edits change nothing");

    session.remove_mcp_server("docs").await.expect("remove");
    assert!(servers().await.is_empty());
}

// ---------------------------------------------------------------------------
// Host queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn host_review_uses_the_session_context_without_touching_history() {
    let (config, captured) = scripted(vec![]);
    let (agent, _) = legacy_agent(Model::simulated_with_config(config), vec![], None);
    let session = Engine::new().create(agent);
    assert!(session.run("original request").await.unwrap().success);
    let events = event_count(&session).await;
    let messages = session.history().page().await.unwrap().messages.len();

    let review = session
        .complete_out_of_band(Some("Review strictly."), "Was the request completed?")
        .await
        .expect("review");
    assert_eq!(review, "done");
    let sent = last_call(&captured);
    assert!(sent.contains("original request"));
    assert!(sent.contains("Review strictly."));
    assert!(sent.contains("Was the request completed?"));

    let context = session.command_context().await.unwrap();
    assert_eq!(context.messages.len(), messages);
    assert_eq!(event_count(&session).await, events);
    assert_eq!(
        session.history().page().await.unwrap().messages.len(),
        messages
    );
}

// ---------------------------------------------------------------------------
// Coherent hosting: the same controls commit through authority operations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coherent_sessions_commit_controls_through_the_hosting_authority() {
    let (config, captured) = scripted(vec![]);
    let efforts = Arc::new(Mutex::new(Vec::new()));
    let mut config = config;
    config.effort_capture = Some(efforts.clone());
    let agent = Agent::builder()
        .instructions("x")
        .model(Model::simulated_with_config(config))
        .build()
        .unwrap();
    // The default Engine hosts sessions coherently in memory.
    let engine = Engine::new();
    engine.register_capability(Extension::default()).unwrap();
    let session = engine.create(agent);

    assert!(session.commands().await.unwrap().is_empty());
    let change = session
        .activate_capability(CapabilityRef::new("hot_extension"))
        .await
        .expect("activate");
    assert!(change.changed && change.active);
    assert!(has_tool(&session, "ext_tool").await);
    assert_eq!(session.commands().await.unwrap()[0].name, "ext");
    let output = session.run_command("/ext", None).await.expect("run");
    assert!(output.message.contains("instructions_have_marker=true"));

    session.set_model("llmsim-large").await.expect("model");
    session
        .set_reasoning_effort(Some(ReasoningEffort::Low))
        .await
        .expect("effort");
    assert!(matches!(
        session.set_model(Model::on("missing_provider", "x")).await,
        Err(SessionControlError::Rejected(_))
    ));
    assert_eq!(session.inspect().await.unwrap().model.model, "llmsim-large");

    assert!(session.run("with extension").await.unwrap().success);
    assert!(last_call(&captured).contains(EXT_MARKER));
    assert_eq!(
        efforts.lock().unwrap().last().unwrap().as_deref(),
        Some("low")
    );

    session
        .deactivate_capability("hot_extension")
        .await
        .unwrap();
    assert!(!has_tool(&session, "ext_tool").await);
    assert!(session.run("without extension").await.unwrap().success);
    assert!(!last_call(&captured).contains(EXT_MARKER));
}

#[tokio::test]
async fn coherent_controls_survive_engine_reconstruction() {
    use everruns::hosting::{AgentBuild, MemoryHosting};
    let build = AgentBuild::new("assistant", "controls-v1");
    let agent = || {
        Agent::builder()
            .instructions("x")
            .model(Model::simulated("ok"))
            .build()
            .unwrap()
    };
    let profile = MemoryHosting::default();
    let first = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), agent())
        .build();
    first.register_capability(Extension::default()).unwrap();
    let id = SessionId::new();
    let session = first.create_registered(id, &build).await.unwrap();
    session
        .activate_capability(CapabilityRef::new("hot_extension"))
        .await
        .unwrap();
    session.set_model("llmsim-large").await.unwrap();
    drop(session);
    drop(first);

    let second = Engine::builder()
        .hosting(profile)
        .register(build, agent())
        .build();
    // Capability code is never deserialized: the host registers it again.
    second.register_capability(Extension::default()).unwrap();
    let resumed = second.resume(id).await.unwrap();
    assert!(has_tool(&resumed, "ext_tool").await);
    assert_eq!(resumed.inspect().await.unwrap().model.model, "llmsim-large");
    assert!(resumed.run("after restart").await.unwrap().success);
    assert!(matches!(
        resumed.deactivate_capability("hot_extension").await,
        Ok(change) if change.changed
    ));
}
