//! External embedding host (EVE-1296): a curated host capability with a
//! pre-tool guard and a model-view contribution, composed into an Engine
//! Session through `everruns::hosting::services` only. It also hot-installs
//! an extension and drives it through the Session control surface (EVE-1297),
//! including an explicit completion-review request (EVE-1303), and attaches
//! Yolop-style request metadata and OpenRouter attribution that must reach the
//! actual provider request (EVE-1302).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use everruns::hosting::services::{
    Capability, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest, HostServices, ModelViewContext, ModelViewProvider, PolicyGate,
    PreToolUseDecision, PreToolUseHook, RequestMetadata, Result, RuntimeMessage, Tool, ToolCall,
    ToolContext, ToolDefinition, ToolExecutionResult, async_trait,
};
use serde_json::{Value, json};

/// Marker the model view adds to every provider-bound conversation.
pub const VIEW_MARKER: &str = "EXTERNAL_HOST_VIEW";
/// Reason the guard gives for a blocked call.
pub const BLOCK_REASON: &str = "external host blocks production deploys";

/// A side-effecting tool: counts every deploy it actually performs.
pub struct Deploy {
    performed: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Tool for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }

    fn description(&self) -> &str {
        "Deploy the current build to an environment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "environment": { "type": "string" } },
            "required": ["environment"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let environment = arguments["environment"].as_str().unwrap_or_default();
        self.performed
            .lock()
            .expect("deploy log")
            .push(environment.to_string());
        ToolExecutionResult::success(json!({ "deployed": environment }))
    }
}

/// Blocks production deploys before dispatch.
pub struct NoProduction;

#[async_trait]
impl PreToolUseHook for NoProduction {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _definition: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.name == "deploy" && tool_call.arguments["environment"] == "production" {
            return PreToolUseDecision::Block {
                tool_call,
                reason: BLOCK_REASON.into(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

/// Appends the host's view marker and counts how often it ran.
pub struct MarkerView(pub Arc<AtomicUsize>);

impl ModelViewProvider for MarkerView {
    fn apply_model_view(
        &self,
        mut messages: Vec<RuntimeMessage>,
        _config: &Value,
        _context: &ModelViewContext<'_>,
    ) -> Vec<RuntimeMessage> {
        self.0.fetch_add(1, Ordering::SeqCst);
        messages.push(RuntimeMessage::user(VIEW_MARKER));
        messages
    }
}

/// The host's curated deploy capability.
pub struct DeployPolicy {
    /// Environments a deploy actually ran against.
    pub performed: Arc<Mutex<Vec<String>>>,
    /// Number of model-view applications.
    pub views: Arc<AtomicUsize>,
}

impl Capability for DeployPolicy {
    fn id(&self) -> &str {
        "external_deploy_policy"
    }

    fn name(&self) -> &str {
        "Deploy policy"
    }

    fn description(&self) -> &str {
        "Deploys with a host guard."
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("Deploy to staging only.")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(Deploy {
            performed: self.performed.clone(),
        })]
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(PolicyGate(NoProduction))]
    }

    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        Some(Arc::new(MarkerView(self.views.clone())))
    }
}

/// An extension installed after startup, with one slash command.
pub struct Extension;

#[async_trait]
impl Capability for Extension {
    fn id(&self) -> &str {
        "external_extension"
    }

    fn name(&self) -> &str {
        "External extension"
    }

    fn description(&self) -> &str {
        "Installed while the host runs."
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("EXTERNAL_EXTENSION_ACTIVE")
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "hello".into(),
            description: "Greet from the extension.".into(),
            source: CommandSource::System,
            args: Vec::new(),
        }]
    }

    async fn execute_command(
        &self,
        _request: &ExecuteCommandRequest,
        context: &CommandExecutionContext,
    ) -> Result<CommandResult> {
        let turn = context.host.turn_context().await?;
        Ok(CommandResult {
            success: true,
            message: format!("hello from {}", turn.model),
            error_code: None,
            error_fields: None,
        })
    }
}

/// Host services carrying the deploy policy.
pub fn services(policy: DeployPolicy) -> HostServices {
    HostServices::new().capability(policy)
}

/// The five values Yolop attaches to every model call: its app name and
/// version, the Everruns core version, and OpenRouter attribution.
pub fn yolop_metadata() -> RequestMetadata {
    RequestMetadata::new()
        .entry("app", "yolop")
        .entry("yolop_version", "0.42.0")
        .entry("everruns_core_version", "0.1.0")
        .openrouter_attribution("https://github.com/everruns/yolop", "Yolop")
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::{Agent, Engine, LlmSimConfig, Model};

    #[tokio::test]
    async fn guard_and_model_view_run_through_an_engine_session() {
        let performed = Arc::new(Mutex::new(Vec::new()));
        let views = Arc::new(AtomicUsize::new(0));
        let deploy = |id: &str, environment: &str| ToolCall {
            id: id.into(),
            name: "deploy".into(),
            arguments: json!({ "environment": environment }),
        };
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut simulator = LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
            vec![deploy("staging", "staging"), deploy("prod", "production")],
            vec![],
        ]);
        simulator.message_capture = Some(captured.clone());
        let agent = Agent::builder()
            .instructions("Deploy when asked.")
            .model(Model::simulated_with_config(simulator))
            .host_services(services(DeployPolicy {
                performed: performed.clone(),
                views: views.clone(),
            }))
            .build()
            .expect("valid agent");
        let session = Engine::new().create(agent);

        let context = session.inspect().await.expect("context");
        assert!(context.tools.iter().any(|tool| tool.name == "deploy"));
        assert!(context.instructions.contains("Deploy to staging only."));

        let turn = session.run("deploy everywhere").await.expect("turn");
        assert!(turn.success, "{:?}", turn.error);
        assert_eq!(
            *performed.lock().expect("deploy log"),
            vec!["staging".to_string()],
            "the guard stopped the production deploy before it ran"
        );
        assert!(views.load(Ordering::SeqCst) >= 2, "one view per model call");
        let calls = captured.lock().expect("captured calls").clone();
        let last = calls.last().expect("a second model call");
        let text = last
            .iter()
            .map(|message| message.content.to_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(VIEW_MARKER));
        assert!(text.contains(BLOCK_REASON));
    }

    #[tokio::test]
    async fn a_hot_installed_extension_runs_through_session_controls() {
        use everruns::{CapabilityRef, ReasoningEffort, SessionControlError};

        let agent = Agent::builder()
            .instructions("Help.")
            .model(Model::simulated("done"))
            .build()
            .expect("valid agent");
        let engine = Engine::new();
        let session = engine.create(agent);
        assert!(session.commands().await.expect("commands").is_empty());

        engine.register_capability(Extension).expect("register");
        let change = session
            .activate_capability(CapabilityRef::new("external_extension"))
            .await
            .expect("activate");
        assert!(change.active);
        let commands = session.commands().await.expect("commands");
        assert_eq!(commands[0].name, "hello");
        let output = session.run_command("/hello", None).await.expect("run");
        assert_eq!(output.message, "hello from llmsim-model");
        assert!(
            session
                .command_context()
                .await
                .expect("context")
                .instructions
                .contains("EXTERNAL_EXTENSION_ACTIVE")
        );

        session.set_model("llmsim-large").await.expect("model");
        session
            .set_reasoning_effort(Some(ReasoningEffort::Medium))
            .await
            .expect("effort");
        assert!(matches!(
            session.set_model(Model::on("unknown", "x")).await,
            Err(SessionControlError::Rejected(_))
        ));
        assert!(session.run("hi").await.expect("turn").success);
        let review = session
            .complete_out_of_band(None, "done?")
            .await
            .expect("review");
        assert_eq!(review, "done");

        session
            .deactivate_capability("external_extension")
            .await
            .expect("deactivate");
        assert!(session.commands().await.expect("commands").is_empty());
    }

    mod request_metadata {
        use std::collections::{BTreeMap, HashMap};
        use std::sync::{Arc, Mutex};

        use everruns::hosting::services::{HostServices, async_trait};
        use everruns::hosting::spi::HostBackends;
        use everruns::llm::Message;
        use everruns::{
            Agent, AgentLoopError, ChatDriver, Engine, LlmCallConfig, LlmResponseStream,
            LlmStreamEvent, Provider, ProviderEndpoint, Session,
        };

        use super::super::yolop_metadata;

        type Requests = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

        /// A host-owned driver that records what reaches the provider.
        #[derive(Clone, Default)]
        struct RecordingDriver(Requests);

        #[async_trait]
        impl ChatDriver for RecordingDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<Message>,
                config: &LlmCallConfig,
            ) -> std::result::Result<LlmResponseStream, AgentLoopError> {
                self.0
                    .lock()
                    .expect("requests")
                    .push((config.model.clone(), config.metadata.clone()));
                Ok(Box::pin(futures::stream::iter([
                    Ok(LlmStreamEvent::TextDelta("recorded".into())),
                    Ok(LlmStreamEvent::Done(Box::default())),
                ])))
            }
        }

        // Yolop's custom-backend lane, deprecated until EVE-1298 retires it.
        #[allow(deprecated)]
        fn agent(driver: &RecordingDriver, backends: HostBackends) -> Agent {
            Agent::builder()
                .instructions("Answer briefly.")
                .provider(Provider::new("yolop-gateway", driver.clone()))
                .model("gateway-model")
                .backends(backends)
                .host_services(HostServices::new().request_metadata(yolop_metadata()))
                .build()
                .expect("valid agent")
        }

        fn assert_last_request(driver: &RecordingDriver, session: &Session, stage: &str) {
            let (model, metadata) = driver
                .0
                .lock()
                .expect("requests")
                .last()
                .cloned()
                .expect("a provider request");
            assert_eq!(model, "gateway-model", "{stage}: provider selection");
            for (key, value) in yolop_metadata().iter() {
                assert_eq!(
                    metadata.get(key).map(String::as_str),
                    Some(value),
                    "{stage}: {key}"
                );
            }
            assert_eq!(
                metadata.get("session_id"),
                Some(&session.session_id().to_string()),
                "{stage}: runtime-owned correlation"
            );
        }

        async fn run(driver: &RecordingDriver, session: &Session, stage: &str) {
            let turn = session.run(stage).await.expect("turn");
            assert!(turn.success, "{stage}: {:?}", turn.error);
            assert_last_request(driver, session, stage);
        }

        #[tokio::test]
        async fn yolop_metadata_reaches_provider_requests_across_rebuilds() {
            let driver = RecordingDriver::default();
            let backends = HostBackends::in_memory();
            let expected: BTreeMap<String, String> = yolop_metadata()
                .iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect();

            let engine = Engine::new();
            let session = engine.create(agent(&driver, backends.clone()));
            let id = session.session_id();
            run(&driver, &session, "first run").await;
            assert_eq!(
                session.inspect().await.expect("inspect").request_metadata,
                expected
            );
            session
                .complete_out_of_band(None, "review")
                .await
                .expect("completion");
            assert_last_request(&driver, &session, "completion");
            drop(session);

            let resumed = engine.resume(id).await.expect("resume");
            run(&driver, &resumed, "resume").await;
            drop(resumed);

            // Process restart: a new Engine and a rebuilt Agent over the
            // host's stores reconstruct the runtime.
            let restarted = Engine::new();
            restarted
                .attach(id, agent(&driver, backends))
                .await
                .expect("attach");
            let reconstructed = restarted.resume(id).await.expect("resume rebuilt");
            run(&driver, &reconstructed, "reconstruction").await;
            assert_eq!(
                reconstructed
                    .command_context()
                    .await
                    .expect("context")
                    .request_metadata,
                expected
            );
        }
    }

    // -----------------------------------------------------------------------
    // A completion controller's explicit review request (EVE-1303)
    // -----------------------------------------------------------------------

    use everruns::llm::{Message, MessageRole};
    use everruns::{
        AgentLoopError, ChatDriver, CompletionRequest, LlmCallConfig, LlmResponseStream,
        LlmStreamEvent, Provider, ProviderEndpoint,
    };

    const REVIEW_PROMPT: &str = "Decide whether the task is complete. Answer COMPLETE or MORE.";

    /// What one provider call received.
    #[derive(Clone, Debug)]
    struct Recorded {
        model: String,
        system: Vec<String>,
        messages: Vec<String>,
        tools: usize,
        metadata: std::collections::HashMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct RecordingDriver(Arc<Mutex<Vec<Recorded>>>);

    #[async_trait]
    impl ChatDriver for RecordingDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &ProviderEndpoint,
            messages: Vec<Message>,
            config: &LlmCallConfig,
        ) -> std::result::Result<LlmResponseStream, AgentLoopError> {
            let text = |role: bool| {
                messages
                    .iter()
                    .filter(|message| (message.role == MessageRole::System) == role)
                    .map(|message| message.content.to_text())
                    .collect::<Vec<_>>()
            };
            self.0.lock().expect("recorded calls").push(Recorded {
                model: config.model.clone(),
                system: text(true),
                messages: text(false),
                tools: config.tools.len(),
                metadata: config.metadata.clone(),
            });
            let reply = if config.metadata.contains_key("purpose") {
                "COMPLETE"
            } else {
                "done"
            };
            Ok(Box::pin(futures::stream::iter([
                Ok(LlmStreamEvent::TextDelta(reply.into())),
                Ok(LlmStreamEvent::Done(Box::default())),
            ])))
        }
    }

    /// The host's own reduction of session context to bounded evidence.
    fn review_evidence(context: &everruns::SessionContext) -> String {
        let mut evidence = format!("messages={}", context.messages.len());
        evidence.truncate(256);
        evidence
    }

    #[tokio::test]
    #[allow(deprecated)] // The legacy facade lane, until its retirement (EVE-1298).
    async fn a_completion_review_sends_exactly_the_host_envelope() {
        for legacy in [false, true] {
            let driver = RecordingDriver::default();
            let mut builder = Agent::builder()
                .instructions("AGENT_INSTRUCTIONS")
                .provider(Provider::new("acme", driver.clone()))
                .model("acme-1");
            if legacy {
                builder = builder.backends(everruns::hosting::spi::HostBackends::in_memory());
            }
            let session = Engine::new().create(builder.build().expect("valid agent"));
            assert!(session.run("ship the fix").await.expect("turn").success);
            session.set_model("acme-2").await.expect("live model");
            let history = session.history().page().await.expect("history").messages;
            let events = session.events_after(0).await.expect("events").len();

            let context = session.command_context().await.expect("context");
            let evidence = review_evidence(&context);
            let verdict = session
                .complete(
                    CompletionRequest::new()
                        .system_prompt(REVIEW_PROMPT)
                        .user(evidence.clone())
                        .purpose("completion_review"),
                )
                .await
                .expect("review");
            assert_eq!(verdict, "COMPLETE");

            let review = driver.0.lock().expect("recorded").last().cloned().unwrap();
            assert_eq!(review.system, [REVIEW_PROMPT]);
            assert_eq!(review.messages, [evidence]);
            assert_eq!(review.tools, 0);
            assert_eq!(review.metadata["purpose"], "completion_review");
            assert_eq!(review.metadata["session_id"], session.id());
            assert_eq!(review.model, "acme-2", "the session's current model");

            // An unavailable selection changes nothing and never falls back.
            assert!(session.set_model(Model::on("other", "x")).await.is_err());
            assert!(matches!(
                session
                    .complete(
                        CompletionRequest::new()
                            .user("x")
                            .metadata("session_id", "spoof")
                    )
                    .await,
                Err(everruns::SessionControlError::Rejected(_))
            ));
            session
                .complete(
                    CompletionRequest::new()
                        .user("again")
                        .purpose("completion_review"),
                )
                .await
                .expect("review");
            assert_eq!(
                driver.0.lock().expect("recorded").last().unwrap().model,
                "acme-2"
            );

            assert_eq!(
                session.history().page().await.expect("history").messages,
                history,
                "canonical history unchanged"
            );
            assert_eq!(session.events_after(0).await.expect("events").len(), events);
        }
    }
}
