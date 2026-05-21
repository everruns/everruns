# everruns-runtime

Public in-process runtime for embedding Everruns harnesses.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It runs
Everruns sessions in your process with in-memory stores by default, while using
the same core `input -> reason -> act` flow as worker-backed execution.

## Quick Example

```rust
use everruns_core::{
    CapabilityRegistry, DriverRegistry, LlmProviderType, ModelWithProvider, PlatformDefinition,
};
use everruns_core::capabilities::TestMathCapability;
use everruns_runtime::InProcessRuntimeBuilder;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut capabilities = CapabilityRegistry::new();
capabilities.register(TestMathCapability);

let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());
let runtime = InProcessRuntimeBuilder::new()
    .platform_definition(platform)
    .llm_sim(everruns_core::llmsim_driver::LlmSimConfig::fixed("4"))
    .default_model(ModelWithProvider {
        model: "llmsim-model".into(),
        provider_type: LlmProviderType::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
    })
    .single_session(|s| {
        s.harness("math", "You are a calculator.")
            .with_capability("test_math")
            .agent("math-agent", "Use tools when useful.")
            .session_title("Math example")
    })
    .build()
    .await?;

let session_id = runtime.default_session_id().expect("single_session id");
let result = runtime.run_text_turn(session_id, "What is 2 + 2?").await?;

assert!(result.success);
# Ok(())
# }
```

Runnable examples:

```text
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
```

## What It Provides

- `InProcessRuntimeBuilder` for embedding sessions
- In-memory stores for local and test usage
- Real-disk workspace wiring for file-backed agents
- Host phase helpers reused by durable and server-backed execution

## License

MIT. See the repository-level `LICENSE` file.
