//! Runs the fleet host: one turn where the model administers this application
//! through `everruns ...` in its own shell.
//!
//! Always a real model: there is no simulated mode, because a simulator told
//! to emit the commands we hoped for demonstrates nothing about whether an
//! agent finds them.
//!
//! ```text
//! ANTHROPIC_API_KEY=... cargo run -p everruns-framework-cli-host
//! OPENAI_API_KEY=...    cargo run -p everruns-framework-cli-host
//!
//! # Any OpenAI-compatible endpoint (OpenRouter, Fireworks, vLLM, a gateway)
//! OPENAI_API_KEY=... OPENAI_BASE_URL=https://openrouter.ai/api/v1 \
//!   OPENAI_MODEL=openai/gpt-4o-mini cargo run -p everruns-framework-cli-host
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
use everruns_provider::driver_registry::DriverRegistry;
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
                // The bash tool returns a JSON envelope. Print stderr and a
                // non-zero exit too: a command that failed is the interesting
                // part of a transcript, and printing only stdout renders it as
                // a blank, which reads like the CLI said nothing at all.
                let rendered = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .map(|value| {
                        let field = |name: &str| {
                            value
                                .get(name)
                                .and_then(|field| field.as_str())
                                .unwrap_or_default()
                                .to_string()
                        };
                        let exit_code = value.get("exit_code").and_then(|code| code.as_i64());
                        let mut out = field("stdout");
                        let stderr = field("stderr");
                        if !stderr.trim().is_empty() {
                            out.push_str(&stderr);
                        }
                        if out.trim().is_empty() {
                            out = match exit_code {
                                Some(0) | None => "(no output)".to_string(),
                                Some(code) => format!("(no output, exit {code})"),
                            };
                        } else if !matches!(exit_code, Some(0) | None) {
                            out.push_str(&format!("[exit {}]", exit_code.unwrap_or_default()));
                        }
                        out
                    })
                    .unwrap_or(text);
                for line in rendered.lines().take(20) {
                    println!("  {line}");
                }
                println!();
            }
            _ => {}
        }
        Ok(())
    }
}

enum Provider {
    Anthropic(String),
    OpenAi(String),
    /// Any OpenAI-compatible endpoint: OpenRouter, Fireworks, vLLM, a gateway.
    /// Chat Completions rather than Responses, since that is the dialect such
    /// endpoints actually implement.
    OpenAiCompatible {
        key: String,
        base_url: String,
        model: String,
    },
}

/// Picks the provider from `--provider <name>`, else whichever key is set.
///
/// The explicit flag matters in practice: an operator often has both keys
/// exported while only one account can currently serve a request.
fn provider_choice() -> Result<Provider, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let requested = args
        .iter()
        .position(|arg| arg == "--provider")
        .and_then(|index| args.get(index + 1))
        .cloned();

    let key = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    // An OpenAI-compatible endpoint wins when configured, so the example can
    // run against whichever account currently has credit without editing code.
    if let (None, Some(base_url), Some(key)) = (
        requested.as_deref(),
        std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|v| !v.is_empty()),
        key("OPENAI_API_KEY"),
    ) {
        return Ok(Provider::OpenAiCompatible {
            key,
            base_url,
            model: std::env::var("OPENAI_MODEL")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| OPENAI_MODEL.to_string()),
        });
    }

    match requested.as_deref() {
        Some("anthropic") => key("ANTHROPIC_API_KEY")
            .map(Provider::Anthropic)
            .ok_or_else(|| "--provider anthropic needs ANTHROPIC_API_KEY".into()),
        Some("openai") => key("OPENAI_API_KEY")
            .map(Provider::OpenAi)
            .ok_or_else(|| "--provider openai needs OPENAI_API_KEY".into()),
        Some(other) => Err(format!("unknown provider {other:?}; use anthropic or openai").into()),
        None => match (key("ANTHROPIC_API_KEY"), key("OPENAI_API_KEY")) {
            (Some(key), _) => Ok(Provider::Anthropic(key)),
            (None, Some(key)) => Ok(Provider::OpenAi(key)),
            (None, None) => Err(
                "set ANTHROPIC_API_KEY or OPENAI_API_KEY (this example calls a real model)".into(),
            ),
        },
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

    // A real model, always. A simulator can be told to emit the exact shell
    // calls we hope for, which makes the demo a recording of our own script:
    // it proves the CLI resolves, never that an agent found it. Only a live
    // model choosing its own commands demonstrates that.
    builder = match provider_choice()? {
        Provider::Anthropic(key) => builder.provider_with_default_model(
            everruns_anthropic::provider("anthropic", key),
            ANTHROPIC_MODEL,
        ),
        Provider::OpenAi(key) => builder
            .provider_with_default_model(everruns_openai::provider("openai", key), OPENAI_MODEL),
        Provider::OpenAiCompatible {
            key,
            base_url,
            model,
        } => builder.provider_with_default_model(
            everruns_openai::completions_provider("openai", key).base_url(base_url),
            model,
        ),
    };

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
