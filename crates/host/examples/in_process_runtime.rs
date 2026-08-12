//! Compose an in-process execution host directly from `everruns-host`.
//!
//! Ordinary applications use the [`everruns`](https://docs.rs/everruns)
//! Framework instead; this example is for advanced hosts that assemble the
//! platform, capability registry, and per-type builders themselves.
//!
//! Run it:
//!
//! ```text
//! cargo run -p everruns-host --example in_process_runtime
//! ```

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::{CapabilityRegistry, DriverId, ResolvedModel};
use everruns_host::HostComposition;
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;
use everruns_test_support::llmsim_driver::LlmSimConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);

    // Supplying a HostComposition replaces the runtime default capability
    // registry, so register every capability this embedded host will reference.
    let platform = HostComposition::new(capabilities, DriverRegistry::new());

    // Per-type builders are useful when an embedder needs stable ids or
    // separate construction. IDs are generated unless `.id(...)` is called.
    let harness_builder =
        HarnessBuilder::new("math", "You are a math assistant.").capability("test_math");
    let harness_id = harness_builder.harness_id();
    let agent_builder = AgentBuilder::new("math-agent", "Use tools when they help.")
        .display_name("Math Agent")
        .max_iterations(8);
    let agent_id = agent_builder.agent_id();
    let session_builder = SessionBuilder::new(harness_id)
        .agent(agent_id)
        .title("Embedded Math Session");
    let _per_type_builders = (
        harness_builder.build(),
        agent_builder.build(),
        session_builder.build(),
    );

    // The runtime below uses the compact convenience.
    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .llm_sim(
            LlmSimConfig::fixed("Let me calculate that.").with_tool_call_sequence(vec![
                vec![everruns_core::ToolCall {
                    id: "call_mul_1".into(),
                    name: "multiply".into(),
                    arguments: serde_json::json!({"a": 6, "b": 7}),
                }],
                vec![],
            ]),
        )
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .single_session(|s| {
            s.harness("math", "You are a math assistant.")
                .with_capability("test_math")
                .agent("math-agent", "Use tools when they help.")
                .agent_display_name("Math Agent")
                .agent_max_iterations(8)
                .session_title("Embedded Math Session")
        })
        .build()
        .await?;

    let session_id = runtime.default_session_id().expect("single_session id");
    let result = runtime.run_text_turn(session_id, "What is 6 * 7?").await?;

    println!("success: {}", result.success);
    println!("iterations: {}", result.iterations);
    println!("response: {}", result.response);

    Ok(())
}
