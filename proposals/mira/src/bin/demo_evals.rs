//! Demo eval **server**: defines evals in code and serves them over the
//! protocol. This is the shape a user's eval program takes — define evals, call
//! [`mira::serve`]. It does no orchestration; the `mira` CLI host drives it.
//!
//! Run it directly and it waits for protocol JSON on stdin. Normally you let
//! the host launch it:
//!
//! ```bash
//! cargo run --bin mira -- --bin demo_evals run
//! ```

#![allow(clippy::type_complexity)]

use std::future::Future;
use std::pin::Pin;

use mira::scorer::{JudgeFn, contains, model_graded, succeeded, turns_within};
use mira::subject::{RuntimeFactory, RuntimeHandle};
use mira::{Eval, ModelSpec, RuntimeSubject, Sample, Score, Transcript};

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{CapabilityRegistry, PlatformDefinition};
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

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

/// A trivial async judge: passes when the response is non-empty. A real judge
/// would call a cheaper model via its own runtime.
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

/// The model matrix: sim always runs; the real cells advertise as unavailable
/// (and are skipped) unless their API keys are present.
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
async fn main() -> std::io::Result<()> {
    mira::serve(evals()).await
}
