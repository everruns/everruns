# everruns-core

> Core agent abstractions for the Everruns durable agentic harness engine.

[![Crates.io](https://img.shields.io/crates/v/everruns-core.svg)](https://crates.io/crates/everruns-core)
[![Documentation](https://docs.rs/everruns-core/badge.svg)](https://docs.rs/everruns-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-core` defines the provider-neutral contracts that every other Everruns
crate builds on: agents, harnesses, sessions, messages, events, typed IDs,
capabilities and tools, the LLM driver registry, and the context assembly that
drives the `input → reason → act` agent loop. It carries no filesystem, shell,
web-fetch, Lua, MCP-client, concrete HTTP, server, or database runtime of its
own — hosts and focused integration crates wire these abstractions together.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Provider crates such as
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-host`](https://crates.io/crates/everruns-host) depend on these
contracts instead of on server internals.

## Quick Example

```rust
use everruns_core::{CapabilityRegistry, DriverRegistry, PlatformDefinition};
use everruns_core::capabilities::CurrentTimeCapability;

let mut capabilities = CapabilityRegistry::new();
capabilities.register(CurrentTimeCapability);

let drivers = DriverRegistry::new();
let platform = PlatformDefinition::new(capabilities, drivers);

assert!(platform.capability_registry().get("current_time").is_some());
```

## What It Provides

- Agent, harness, session, message, event, and typed ID domain models
- Capability and tool traits for extending what agents can do
- An LLM driver registry and provider-neutral message, tool, and reasoning types
- Context assembly shared by embedded, worker, and server execution paths
- Minimal in-memory store implementations for embedding and prototyping

Environment implementations live in `everruns-integrations-filesystem`,
`everruns-integrations-bashkit`, `everruns-integrations-web-fetch`,
`everruns-integrations-lua`, `everruns-mcp`, and `everruns-http`.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-core)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [The agentic loop](https://docs.everruns.com/explanation/agentic-loop/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
