# everruns-core

Core agent abstractions for Everruns: agents, sessions, events, tools,
capabilities, LLM driver traits, runtime context assembly, and shared domain
types.

This crate is part of the [Everruns](https://everruns.com) ecosystem. Provider
crates such as `everruns-openai` and host crates such as `everruns-runtime`
build on these contracts instead of depending on server internals.

## Quick Example

```rust
use everruns_core::{CapabilityRegistry, DriverRegistry, PlatformDefinition};
use everruns_core::capabilities::TestMathCapability;

let mut capabilities = CapabilityRegistry::new();
capabilities.register(TestMathCapability);

let drivers = DriverRegistry::new();
let platform = PlatformDefinition::new(capabilities, drivers);

assert!(platform.capability_registry().get("test_math").is_some());
```

## What It Provides

- Agent, harness, session, message, event, and typed ID models
- Capability and tool traits
- LLM driver registry and provider-neutral message types
- Context assembly used by embedded, worker, and server execution paths
- In-memory helpers and deterministic LLM simulation for tests

## License

MIT. See the repository-level `LICENSE` file.
