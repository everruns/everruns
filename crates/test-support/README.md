# everruns-test-support

> In-memory loops, test doubles, writable fixtures, and demo capabilities for testing Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-test-support.svg)](https://crates.io/crates/everruns-test-support)
[![Documentation](https://docs.rs/everruns-test-support/badge.svg)](https://docs.rs/everruns-test-support)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-test-support` is the focused test kit for the
[`everruns`](https://crates.io/crates/everruns) Framework and the wider
Everruns ecosystem: a fully in-memory agentic loop, mock/echo/failing test
doubles, writable deterministic fixtures, and fake demo capabilities. The
production-safe simulator driver lives in
[`everruns-llmsim`](https://crates.io/crates/everruns-llmsim); this crate uses
it to keep integration testing ergonomic without making production
compositions depend on testing/demo helpers.

Ordinary applications do not depend on this crate: they use
`everruns::Model::simulated` or `everruns-llmsim` directly. During the 0.18
migration, the 0.17 simulator imports remain re-exported here for source
compatibility; new code should use the owning crate.

## Quick Example

```rust
use everruns_llmsim::LlmSimConfig;
use everruns_test_support::{InMemoryAgenticLoop, TestMathCapability};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = InMemoryAgenticLoop::builder()
    .with_llm_sim(LlmSimConfig::fixed("2 + 2 = 4"))
    .capability(TestMathCapability)
    .build()
    .await?;

let result = agent.run_turn("What is 2 + 2?").await?;
assert!(result.success);
assert!(result.contains("4"));
# Ok(())
# }
```

## What It Provides

- `InMemoryAgenticLoop`, a complete `input → reason → act` loop over
  host-owned in-memory stores and a canonical in-memory event log, for tests
  and prototypes with no database or network (`sim` + `host` features)
- `InMemoryMessageRetriever` / `InMemoryEventEmitter`, writable deterministic
  fixtures for isolated atom and trait tests; hosted loops instead append once
  to an event log and read through `EventHistory`
- `MockProvider`, `MockToolExecutor`, `EchoToolExecutor`,
  `FailingToolExecutor`, test doubles for the core execution traits
- Fake demo capabilities, `FakeAwsCapability`, `FakeCrmCapability`,
  `FakeFinancialCapability`, `FakeWarehouseCapability`, plus
  `TestMathCapability`, `TestWeatherCapability`, `SampleDataCapability`, and
  `NoopCapability` (`fixtures` feature; never registered by production
  compositions)
- 0.18 compatibility re-exports for simulator types and
  `LlmSimRuntimeExt`; `.llm_sim(...)` remains registration-only and
  `.llm_sim_as_default(...)` explicitly selects the simulator. Their
  implementation and recommended direct import live in `everruns-llmsim`.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-test-support)
- [Simulator API reference](https://docs.rs/everruns-llmsim)
- [Framework testing and simulation](https://docs.everruns.com/framework/testing-and-simulation/)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [Everruns documentation](https://docs.everruns.com)

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents.

## License

MIT, see [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
