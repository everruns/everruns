//! End-to-end demo of the prototype against the real `everruns-runtime`.
//!
//! Runs offline with `llmsim` (no API key). The Anthropic/OpenAI matrix cells
//! are skipped automatically unless `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` are
//! set, so this always runs green out of the box:
//!
//! ```bash
//! cargo run --example coding_eval                 # all evals, sim cell
//! cargo run --example coding_eval -- greet        # selective: substring filter
//! cargo run --example coding_eval -- --tag smoke  # selective: by tag
//! cargo run --example coding_eval -- --json out.json
//! ```

#![allow(clippy::type_complexity)]

use std::future::Future;
use std::pin::Pin;

use clap::Parser;

use evals::scorer::{JudgeFn, contains, model_graded, succeeded, turns_within};
use evals::subject::{RuntimeFactory, RuntimeHandle};
use evals::{Eval, ModelSpec, Runner, RuntimeSubject, Sample, Score, Transcript, report};

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{CapabilityRegistry, PlatformDefinition};
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

#[derive(Parser)]
#[command(about = "Prototype eval framework demo")]
struct Args {
    /// Substring filter on the case key `eval/sample@model` (like `cargo test PAT`).
    filter: Option<String>,
    /// Only run samples carrying this tag.
    #[arg(long)]
    tag: Option<String>,
    /// Write the JSON report to this path.
    #[arg(long)]
    json: Option<String>,
}

/// Builds a fresh, minimal runtime per matrix cell. Embedders own this wiring;
/// the framework only needs the resulting `(runtime, session_id)`.
fn runtime_factory() -> RuntimeFactory {
    Box::new(
        |spec: ModelSpec| -> Pin<Box<dyn Future<Output = Result<RuntimeHandle, String>> + Send>> {
            Box::pin(async move {
                let mut registry = DriverRegistry::new();
                everruns_anthropic::register_driver(&mut registry);
                everruns_openai::register_driver(&mut registry);

                let platform = PlatformDefinition::builder()
                    .capability_registry(CapabilityRegistry::new())
                    .driver_registry(registry)
                    .build();

                let runtime = InProcessRuntimeBuilder::new()
                    .platform_definition(platform)
                    .default_model(spec.model.clone())
                    .backends(RuntimeBackends::in_memory())
                    .single_session(|s| {
                        s.harness("eval-harness", "You are a terse assistant.")
                            .harness_display_name("Eval Harness")
                            .agent("eval-agent", "Answer the user directly and briefly.")
                            .agent_display_name("Eval Agent")
                            .session_title("eval session")
                    })
                    .llm_sim(
                        LlmSimConfig::fixed("hello from llmsim — the answer is 42")
                            .with_model("llmsim-eval"),
                    )
                    .build()
                    .await
                    .map_err(|e| e.to_string())?;

                let session_id = runtime
                    .default_session_id()
                    .ok_or_else(|| "runtime created no default session".to_string())?;
                Ok((runtime, session_id))
            })
        },
    )
}

/// A trivial async judge to demonstrate the LLM-as-judge seam without spending
/// tokens: passes when the response is non-empty. A real judge would call a
/// cheaper model via its own runtime.
fn nonempty_judge() -> JudgeFn {
    Box::new(
        |rubric: String, t: Transcript| -> Pin<Box<dyn Future<Output = Score> + Send>> {
            Box::pin(async move {
                if t.final_response.trim().is_empty() {
                    Score::fail("model_graded", format!("empty response (rubric: {rubric})"))
                } else {
                    Score::pass("model_graded", "non-empty response")
                }
            })
        },
    )
}

/// The model matrix: sim always runs; the real cells skip without API keys.
fn matrix() -> Vec<ModelSpec> {
    vec![
        ModelSpec::sim(),
        ModelSpec::anthropic("claude-haiku-4-5"),
        ModelSpec::openai("gpt-5.5"),
    ]
}

fn evals() -> Vec<Eval> {
    vec![
        Eval::new("greet")
            .case("hi", "Say hi and tell me the answer.")
            .subject(RuntimeSubject::new(runtime_factory()))
            .scorer(succeeded())
            .scorer(contains("42"))
            .scorer(turns_within(3))
            .models(matrix())
            .build(),
        Eval::new("judge")
            .sample(Sample::new("smoke", "Anything at all.").tag("smoke"))
            .subject(RuntimeSubject::new(runtime_factory()))
            .scorer(succeeded())
            .scorer(model_graded("Is the answer responsive?", nonempty_judge()))
            .models(matrix())
            .build(),
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut runner = Runner::new().filter(args.filter).tag(args.tag);
    for eval in evals() {
        runner = runner.add(eval);
    }

    let report = runner.run().await;
    report::print_summary(&report);

    if let Some(path) = args.json {
        let json = report::to_json(&report);
        std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
        println!("\nwrote {path}");
    }

    std::process::exit(if report.all_passed() { 0 } else { 1 });
}
