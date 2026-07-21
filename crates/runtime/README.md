# everruns-runtime

> The agentic runtime — build agents in Rust by embedding the Everruns agentic framework directly in your process. No server, worker, or database required.

[![Crates.io](https://img.shields.io/crates/v/everruns-runtime.svg)](https://crates.io/crates/everruns-runtime)
[![Documentation](https://docs.rs/everruns-runtime/badge.svg)](https://docs.rs/everruns-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-runtime` is the agentic runtime at the core of Everruns — the
entrypoint to the Everruns agentic framework. It runs an agent harness entirely
inside your own process — with in-memory stores by default — using the same
`input → reason → act` loop as worker- and server-backed execution, but without
the durable engine, gRPC worker boundary, or control-plane server. It is the
recommended starting point for building agents, trying Everruns, and writing
tests.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. The runtime builds on the
contracts in [`everruns-core`](https://crates.io/crates/everruns-core) and pairs
with provider crates such as
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-anthropic`](https://crates.io/crates/everruns-anthropic).

## Quick Example

```rust
use everruns_core::{
    CapabilityRegistry, DriverId, DriverRegistry, PlatformDefinition, ResolvedModel,
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
    .default_model(ResolvedModel {
        model: "llmsim-model".into(),
        provider_type: DriverId::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
        provider_metadata: None,
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
assert_eq!(result.stop_reason, everruns_runtime::TurnStopReason::EndTurn);
# Ok(())
# }
```

The example above uses the built-in deterministic LLM simulator so it runs with
no API key. Swap in a real provider (for example
[`everruns-openai`](https://crates.io/crates/everruns-openai)) to talk to a
hosted model.

`InProcessRuntimeBuilder::new()` starts with a runtime-safe built-in capability
registry. If you call `.platform_definition(...)`, that platform becomes
authoritative; start from `CapabilityRegistry::runtime_builtins()` when you want
the default runtime catalog plus your own additions.

Runnable examples ship with the crate:

```text
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
cargo run -p everruns-runtime --example real_disk_file_system_tools
```

## What It Provides

- `InProcessRuntimeBuilder` for declaring platforms, harnesses, agents, and sessions in code
- In-memory stores for local development and tests, with hooks to plug in custom backends
- Real-disk workspace wiring so file-backed agents can read and write an actual directory
- Turn-context inspection before and after a turn executes
- Host-phase helpers reused by durable and server-backed execution

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-runtime)
- [Runtime — embed Everruns in your process](https://docs.everruns.com/features/runtime/)
- [Tutorial: build your first agent](https://docs.everruns.com/tutorials/building-agents-using-sdk/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
