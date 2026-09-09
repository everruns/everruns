//! Runs the fleet host: one turn where the model administers this application
//! through `everruns ...` in its own shell.
//!
//! ```text
//! # Against a real provider
//! OPENAI_API_KEY=... cargo run -p everruns-framework-cli-host
//!
//! # Offline, deterministic, no key required
//! cargo run -p everruns-framework-cli-host -- --offline
//! ```

use std::sync::Arc;

use everruns_core::events::{Event, EventData};
use everruns_core::{ContentPart, InputMessage};
use everruns_framework_cli_host::{Fleet, FleetCommands};
use everruns_host::{
    AgentBuilder, EventSink, EventSinkError, HarnessBuilder, HostBackends, HostComposition,
    InMemorySessionFileSystemFactory, InProcessRuntimeBuilder, SessionBuilder,
};
use everruns_integrations_bashkit::BashkitShellCapability;
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};

const OPENAI_MODEL: &str = "gpt-5.6-terra";
const ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";

const INSTRUCTIONS: &str = "\
You administer this deployment. Its operations are available in your shell as \
`everruns <noun> <verb> [--flags]`; run `everruns --help` to see the nouns and \
`everruns <noun> --help` for a noun's verbs. Use the shell to answer questions \
about the fleet rather than guessing, and report exactly what the commands \
returned.";

const DEFAULT_PROMPT: &str = "List the fleet, then scale the api service to 4 replicas.";

/// Everything after the flags becomes the prompt, so the example can be driven
/// at a mutation (`... scale api to 4 replicas`) and checked against the state
/// printed afterwards.
fn prompt_from_args() -> String {
    let prompt = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ");
    if prompt.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        prompt
    }
}

/// Prints the agent's shell session as it happens.
///
/// Without this the demo shows only the model's final prose, which proves
/// nothing: a model can claim any answer. What is worth seeing is the command
/// the agent typed and the bytes the `everruns` builtin actually returned.
#[derive(Default)]
struct ShellTranscript;

impl EventSink for ShellTranscript {
    fn try_send(&self, event: Event) -> Result<(), EventSinkError> {
        match &event.data {
            EventData::ToolStarted(started) if started.tool_call.name == "bash" => {
                if let Some(commands) = started
                    .tool_call
                    .arguments
                    .get("commands")
                    .and_then(|value| value.as_str())
                {
                    for line in commands.lines() {
                        println!("  $ {line}");
                    }
                }
            }
            EventData::ToolCompleted(completed) if completed.tool_name == "bash" => {
                let text = completed
                    .result
                    .as_ref()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| match part {
                                ContentPart::Text(text) => Some(text.text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default();
                // The bash tool returns a JSON envelope; print the stdout it
                // carries so the transcript reads like the terminal session it
                // actually was.
                let output = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("stdout")
                            .and_then(|out| out.as_str())
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or(text);
                for line in output.lines().take(20) {
                    println!("  {line}");
                }
                println!();
            }
            _ => {}
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let offline = std::env::args().any(|arg| arg == "--offline");
    let fleet = Fleet::with_demo_services();

    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let composition = HostComposition::builder()
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(InMemorySessionFileSystemFactory))
        .build();

    // The seam: hand the runtime this application's commands. A host that
    // supplies none gets no `everruns` builtin at all.
    let source_fleet = fleet.clone();
    let mut builder = InProcessRuntimeBuilder::new()
        .host_composition(composition)
        .backends(HostBackends::in_memory().with_event_sink(Arc::new(ShellTranscript)))
        // The stock shell capability. What makes this application's own
        // operations reachable is the command source below, not a special
        // shell.
        .capability(BashkitShellCapability)
        .with_tool_context_extensions_factory(Arc::new(move |_org, _session| {
            let mut extensions = everruns_core::tool_context::ToolContextExtensions::default();
            extensions.insert(Arc::new(FleetCommands::handle(source_fleet.clone())));
            extensions
        }))
        .harness(
            HarnessBuilder::new("fleet-admin", INSTRUCTIONS)
                .id(harness_id)
                .capability("bashkit_shell")
                .build(),
        )
        .agent(
            AgentBuilder::new("fleet-admin", INSTRUCTIONS)
                .id(agent_id)
                .max_iterations(12)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        );

    if offline {
        // Deterministic: the simulator drives the exact shell calls a model
        // would make, so the example runs in CI without a key.
        builder = builder.llm_sim_as_default(
            LlmSimConfig::fixed("Listed the fleet and scaled api from 2 replicas to 4.")
                .with_tool_call_sequence(vec![
                    vec![ToolCall {
                        id: "call_help".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "commands": "everruns fleet --help" }),
                    }],
                    vec![ToolCall {
                        id: "call_list".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "commands": "everruns fleet list" }),
                    }],
                    vec![ToolCall {
                        id: "call_scale".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({
                            "commands": "everruns fleet scale --name api --replicas 4"
                        }),
                    }],
                    vec![],
                ]),
        );
    } else {
        // Either key works; whichever is present wins, so the example runs
        // wherever the operator already has credentials.
        builder = match (
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        ) {
            (Some(key), _) => builder.provider_with_default_model(
                everruns_anthropic::provider("anthropic", key),
                ANTHROPIC_MODEL,
            ),
            (None, Some(key)) => builder.provider_with_default_model(
                everruns_openai::provider("openai", key),
                OPENAI_MODEL,
            ),
            (None, None) => {
                return Err(
                    "set ANTHROPIC_API_KEY or OPENAI_API_KEY, or pass --offline for the \
                     deterministic run"
                        .into(),
                );
            }
        };
    }

    let runtime = builder.build().await?;

    let prompt = prompt_from_args();
    println!("Prompt: {prompt}\n");
    println!("== Agent's shell session ==");
    let result = runtime
        .run_turn(session_id, InputMessage::user(prompt))
        .await?;

    println!("== Response ==\n{}\n", result.response);

    // Print the real state, not just what the model said about it: a claimed
    // scale and an actual one look identical in prose.
    println!("== Fleet state after the turn ==");
    for name in ["api", "worker", "scheduler"] {
        match fleet.replicas(name) {
            Some(replicas) => println!("  {name}: {replicas}"),
            None => println!("  {name}: (absent)"),
        }
    }

    println!(
        "\nturn success: {} | iterations: {} | tool calls: {}",
        result.success, result.iterations, result.tool_calls_count
    );
    if let Some(error) = &result.error {
        println!("turn error: {error}");
    }
    Ok(())
}
