//! EVE-1296: a host capability composed through `HostServices` presents the
//! same model surface through an Engine Session as through a directly composed
//! Core in-process runtime, which is how embedding hosts ran it before.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use everruns::hosting::services::HostServices;
use everruns::{Agent, Engine, Model};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use everruns_core::command::{CommandDescriptor, CommandSource};
use everruns_core::host::InProcessRuntimeBuilder;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_hooks::{PolicyGate, PreToolUseDecision, PreToolUseHook};
use everruns_core::{Capability, Tool, ToolExecutionResult};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use serde_json::{Value, json};

const ADDITION: &str = "PARITY_CAPABILITY_ADDITION";

struct Probe(Arc<AtomicUsize>);

#[async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "Probe a resource."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "target": { "type": "string", "enum": ["safe", "unsafe"] } },
            "required": ["target"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ToolExecutionResult::success(json!({ "probed": arguments["target"] }))
    }
}

struct SafeOnly;

#[async_trait]
impl PreToolUseHook for SafeOnly {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _definition: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.arguments["target"] == "unsafe" {
            return PreToolUseDecision::Block {
                tool_call,
                reason: "unsafe probes are blocked".into(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

struct ProbeCapability(Arc<AtomicUsize>);

impl Capability for ProbeCapability {
    fn id(&self) -> &str {
        "parity_probe"
    }

    fn name(&self) -> &str {
        "Parity probe"
    }

    fn description(&self) -> &str {
        "A probe with a host guard."
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(ADDITION)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(Probe(self.0.clone()))]
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(PolicyGate(SafeOnly))]
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "probe-status".into(),
            description: "Show probe status.".into(),
            source: CommandSource::System,
            args: Vec::new(),
        }]
    }
}

fn calls() -> Vec<Vec<ToolCall>> {
    vec![
        vec![
            ToolCall {
                id: "a".into(),
                name: "probe".into(),
                arguments: json!({ "target": "safe" }),
            },
            ToolCall {
                id: "b".into(),
                name: "probe".into(),
                arguments: json!({ "target": "unsafe" }),
            },
        ],
        vec![],
    ]
}

#[tokio::test]
async fn host_capability_matches_direct_core_composition() {
    // Direct host composition: the existing embedding lane.
    let direct_effects = Arc::new(AtomicUsize::new(0));
    let runtime = InProcessRuntimeBuilder::new()
        .capability(ProbeCapability(direct_effects.clone()))
        .single_session(|session| {
            session
                .agent("probe-agent", "Probe things.")
                .agent_capability("parity_probe")
        })
        .llm_sim_as_default(LlmSimConfig::fixed("done").with_tool_call_sequence(calls()))
        .build()
        .await
        .unwrap();
    let session_id = runtime.default_session_id().unwrap();
    let direct = runtime.load_context(session_id).await.unwrap();
    let direct_commands = runtime.list_commands(session_id).await.unwrap();
    let direct_turn = runtime.run_text_turn(session_id, "probe").await.unwrap();

    // Engine-owned composition through host services.
    let engine_effects = Arc::new(AtomicUsize::new(0));
    let agent = Agent::builder()
        .instructions("Probe things.")
        .model(Model::simulated_with_config(
            LlmSimConfig::fixed("done").with_tool_call_sequence(calls()),
        ))
        .host_services(HostServices::new().capability(ProbeCapability(engine_effects.clone())))
        .build()
        .unwrap();
    let session = Engine::new().create(agent);
    let context = session.inspect().await.unwrap();
    let turn = session.run("probe").await.unwrap();

    let direct_tool = direct
        .runtime_agent
        .tools
        .iter()
        .find(|tool| tool.name() == "probe")
        .expect("direct tool");
    let engine_tool = context
        .tools
        .iter()
        .find(|tool| tool.name == "probe")
        .expect("engine tool");
    assert_eq!(engine_tool.description, direct_tool.description());
    assert_eq!(&engine_tool.parameters, direct_tool.parameters());

    assert!(direct.runtime_agent.system_prompt.contains(ADDITION));
    assert!(context.instructions.contains(ADDITION));

    let direct_commands = direct_commands
        .into_iter()
        .map(|command| (command.name, command.description))
        .collect::<Vec<_>>();
    let engine_commands = context
        .commands
        .iter()
        .map(|command| (command.name.clone(), command.description.clone()))
        .collect::<Vec<_>>();
    assert_eq!(engine_commands, direct_commands);

    assert!(turn.success, "{:?}", turn.error);
    let _ = direct_turn;
    assert_eq!(direct_effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        engine_effects.load(Ordering::SeqCst),
        direct_effects.load(Ordering::SeqCst),
        "the same calls are allowed and blocked"
    );
}
