#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Runs the fleet host: one turn where the model administers this application
//! through `everruns ...` in its own shell.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-framework-cli-host
//! ```

use std::sync::Arc;

use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::InputMessage;
use everruns_core::host::{
    AgentBuilder, HarnessBuilder, HostComposition, InMemorySessionFileSystemFactory,
    InProcessRuntimeBuilder, SessionBuilder,
};
use everruns_framework_cli_host::{Fleet, FleetCommands};
use everruns_integrations::bashkit::BashkitShellCapability;

const OPENAI_MODEL: &str = "gpt-6.1-sol";
const ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";

const INSTRUCTIONS: &str = "\
You administer this deployment. Its operations are available in your shell as \
`everruns <noun> <verb> [--flags]`; run `everruns --help` to see the nouns and \
`everruns <noun> --help` for a noun's verbs. Use the shell to answer questions \
about the fleet rather than guessing, and report exactly what the commands \
returned.";

const DEFAULT_PROMPT: &str =
    "How many replicas is the api service running, and what other services exist?";

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let builder = InProcessRuntimeBuilder::new()
        .host_composition(composition)
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

    // A real model: OpenAI's gpt-6.1-sol by default, or Anthropic when only
    // that key is present. Tests script the same shell calls with a simulated
    // model in `tests/`; the example never does.
    let builder = match (
        std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty()),
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty()),
    ) {
        (Some(key), _) => builder.provider_with_default_model(
            everruns_drivers::openai::provider("openai", key),
            OPENAI_MODEL,
        ),
        (None, Some(key)) => builder.provider_with_default_model(
            everruns_drivers::anthropic::provider("anthropic", key),
            ANTHROPIC_MODEL,
        ),
        (None, None) => return Err("set OPENAI_API_KEY (or ANTHROPIC_API_KEY)".into()),
    };

    let runtime = builder.build().await?;

    let prompt = prompt_from_args();
    println!("Prompt: {prompt}\n");
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
