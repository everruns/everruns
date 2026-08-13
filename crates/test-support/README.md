# everruns-test-support

> Deterministic LLM simulation, in-memory agent loop, and demo capability fixtures for testing Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-test-support.svg)](https://crates.io/crates/everruns-test-support)
[![Documentation](https://docs.rs/everruns-test-support/badge.svg)](https://docs.rs/everruns-test-support)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-test-support` is the focused test kit for the
[`everruns`](https://crates.io/crates/everruns) Framework and the wider
Everruns ecosystem: the in-process `llmsim` chat driver, a fully in-memory
agentic loop, mock/echo/failing test doubles, and the fake demo capabilities
(AWS/CRM/financial/warehouse, test math/weather, sample-data, noop). It
exists so production crates — `everruns-core` and the provider drivers —
ship none of this by default while deterministic Framework and integration
testing stays ergonomic.

Ordinary applications do not depend on this crate: `everruns::Model::simulated`
re-exports the one simulator convenience the facade needs. Test suites,
examples, and benchmarks depend on it directly.

## Quick Example

```rust
use everruns_test_support::{InMemoryAgenticLoop, LlmSimConfig, TestMathCapability};

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

- `LlmSimDriver` / `LlmSimConfig` — deterministic in-process chat driver:
  fixed, echo, sequence, and scripted multi-turn responses, tool-call
  scripting, latency/TTFT simulation, error injection, and effort/message
  capture sinks (`sim` feature)
- `InMemoryAgenticLoop` — a complete `input → reason → act` loop over
  host-owned in-memory stores and a canonical in-memory event log, for tests
  and prototypes with no database or network (`sim` + `host` features)
- `InMemoryMessageRetriever` / `InMemoryEventEmitter` — writable deterministic
  fixtures for isolated atom and trait tests; hosted loops instead append once
  to an event log and read through `EventHistory`
- `MockProvider`, `MockToolExecutor`, `EchoToolExecutor`,
  `FailingToolExecutor` — test doubles for the core execution traits
- Fake demo capabilities — `FakeAwsCapability`, `FakeCrmCapability`,
  `FakeFinancialCapability`, `FakeWarehouseCapability`, plus
  `TestMathCapability`, `TestWeatherCapability`, `SampleDataCapability`, and
  `NoopCapability` (`fixtures` feature; never registered by production
  compositions)
- `LlmSimRuntimeExt` — `.llm_sim(...)` sugar for
  `everruns_host::InProcessRuntimeBuilder` (`host` feature)

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-test-support)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [Everruns documentation](https://docs.everruns.com)

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## License

MIT — see [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
