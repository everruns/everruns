# everruns-llmsim

> Deterministic, offline LLM simulation for Everruns agents and runtimes.

`everruns-llmsim` provides the production-safe simulator driver used by the
Everruns Framework for fixed, echo, sequence, and scripted responses. It runs
in process without credentials or network access and preserves deterministic
tool calls, injected failures, latency controls, and request capture.

It is part of the [Everruns](https://everruns.com) ecosystem, builds on the
provider contracts in `everruns-provider`, and optionally pairs with
`everruns-host` for runtime-builder integration.

## Quick start

```rust
use everruns_llmsim::{LlmSimConfig, LlmSimDriver};

let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello from the simulator."));
```

Framework applications can keep the shorter facade API:

```rust
use everruns::Model;

let model = Model::simulated("Hello from the simulator.");
```

## What It Provides

- Fixed, echo, lorem, sequence, and scripted multi-turn responses
- Deterministic text, tool-call, mixed, error, and stalled-stream turns
- Latency, metadata, reasoning-effort, and provider-message controls
- Driver registry helpers for product and embedded runtimes
- Optional `host` feature with registration-only `.llm_sim(...)` and explicit
  `.llm_sim_as_default(...)` runtime-builder integration

## Documentation

- [Framework testing and simulation](https://docs.everruns.com/framework/testing-and-simulation/)
- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [API reference](https://docs.rs/everruns-llmsim)

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE)
