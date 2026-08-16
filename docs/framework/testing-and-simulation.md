---
title: Testing and Simulation
description: Test Framework applications deterministically without network access or provider credentials.
---

`Model::simulated` is the default testing tool. It uses the normal provider
resolution and execution path while returning a fixed response locally. Its
implementation comes from the publishable, production-safe
`everruns-llmsim` crate; the Framework does not depend on test-support code.

```rust
use everruns::{Agent, Engine, Model};

# #[tokio::test]
# async fn agent_follows_the_application_flow() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Return the configured result.")
    .model(Model::simulated("approved"))
    .build()?;

let session = Engine::new().create(agent);
let turn = session.send_and_wait("Review this.").await?;
assert!(turn.success);
assert_eq!(turn.response, "approved");
# Ok(())
# }
```

Useful test layers are:

1. Build-time validation tests for agent, tool, model, MCP, and compaction configuration.
2. Offline session tests with `Model::simulated`.
3. Context assertions through `Session::inspect`.
4. Event/cancellation tests through the public session API.
5. A small opt-in live-provider suite for protocol integration.

Keep normal tests credential-free and deterministic. Do not make a live model's
wording or tool choice a unit-test oracle. Temporary directories should own
workspace and local-state tests so they do not read or modify developer data.

The runnable programs in [Framework examples](/framework/examples/) are also compiled
in CI using only the public `everruns` facade.

## Scripted and low-level simulation

Use `Model::simulated_with_config` when an application test needs multiple
assistant turns, deterministic tool calls, an injected provider error, or
request capture:

```rust
use everruns::{Agent, LlmSimConfig, Model};
use everruns_llmsim::{SimToolCall, SimTurn};

let simulation = LlmSimConfig::scripted(vec![
    SimTurn::ToolCalls(vec![SimToolCall {
        name: "lookup".into(),
        arguments: serde_json::json!({"id": 7}),
        id: Some("call_lookup".into()),
    }]),
    SimTurn::Assistant("approved".into()),
]);

let agent = Agent::builder()
    .instructions("Use lookup, then report the result.")
    .model(Model::simulated_with_config(simulation))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Advanced hosts depend on `everruns-llmsim` with its `host` feature for
`LlmSimRuntimeExt`. The `.llm_sim(...)` method registers the provider without
changing model selection; `.llm_sim_as_default(...)` explicitly selects it
when no default was already configured. Use `everruns-test-support` only for
testing/demo helpers such as its in-memory agentic loop, writable fixtures,
test doubles, and fake capabilities. The test-support simulator re-exports
exist only as a 0.18 migration bridge for 0.17 import paths.
