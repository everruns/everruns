//! External embedding host (EVE-1296): a curated host capability with a
//! pre-tool guard and a model-view contribution, composed into an Engine
//! Session through `everruns::hosting::services` only. It also hot-installs
//! an extension and drives it through the Session control surface (EVE-1297).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use everruns::hosting::services::{
    Capability, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest, HostServices, ModelViewContext, ModelViewProvider, PolicyGate,
    PreToolUseDecision, PreToolUseHook, Result, RuntimeMessage, Tool, ToolCall, ToolContext,
    ToolDefinition, ToolExecutionResult, async_trait,
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
}
