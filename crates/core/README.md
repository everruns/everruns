# everruns-core

> Core agent abstractions for the Everruns durable agentic harness engine.

[![Crates.io](https://img.shields.io/crates/v/everruns-core.svg)](https://crates.io/crates/everruns-core)
[![Documentation](https://docs.rs/everruns-core/badge.svg)](https://docs.rs/everruns-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-core` defines the provider-neutral contracts that every other Everruns
crate builds on: agents, harnesses, sessions, messages, events, capabilities
and tools, plus pure snapshot/context transformations and narrow effect
contracts for the `input → reason → act` execution kernel. The concrete phase
algorithms live in `everruns-engine`. Core carries no store
or provider-loading orchestration, filesystem, shell,
web-fetch, Lua, MCP-client, concrete HTTP, server, or database runtime of its
own — hosts and focused integration crates wire these abstractions together.
Knowledge Bases and Indexes, Memories, delegation, schedules/tasks, user hooks,
and platform-management capabilities live in `everruns-platform` and are not
advertised by the Framework preset.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. Provider crates such as
[`everruns-openai`](https://crates.io/crates/everruns-openai) and
[`everruns-host`](https://crates.io/crates/everruns-host) depend on these
contracts instead of on server internals.

## Quick Example

```rust
use everruns_core::CapabilityRegistry;
use everruns_provider::DriverRegistry;

let capabilities = CapabilityRegistry::new();
let drivers = DriverRegistry::new();

assert!(capabilities.is_empty());
assert!(drivers.registered_providers().is_empty());
```

Core owns the capability registry and execution contracts; provider owns the
driver registry. Neither owns the bundle that selects a deployment's shape. An embedder assembles those into an
`everruns_host::HostComposition` and hands it to the runtime.

Core's default feature set is empty. `openapi` and
`tree-sitter-outlines` are explicit opt-ins, and concrete protocol drivers and
TLS/HTTP setup belong to `everruns-provider` and their host startup owners.

`everruns-core` does not register a policy catalog. Applications that want the
standard backend-neutral policies compose `everruns-builtins`; environment and
hosted capabilities come from their owning integration or product crates.

## What It Provides

- Agent, harness, session, message, and event domain models
- Capability registry/execution contracts and tool traits for extensions
- Provider-neutral execution context and effect contracts
- Secret-free execution snapshots and pure resolved-context transformations
- Neutral capability collection hooks and type-keyed host-service extensions
- Read-only storage traits and canonical event/message contracts

Per-turn effect contracts are organized by concern: `execution_loading`,
`tool_execution`, `tool_context`, `session_files`, `session_services`,
`durability`, `event_emitter`, `provider_resolution`, `image_services`,
`connection_services`, and `delegation_services`. The crate intentionally has no catch-all `traits`
module. Deployment composition contracts, including
`SessionFileSystemFactory`, live in `everruns-host`.

Store-backed snapshot loading, lifecycle/dependency probing, message filtering,
provider configuration and driver construction, context inspection, and
`StoreCommandHost` live in `everruns-host`. Engine reason execution accepts an
already assembled credential-safe context or the narrow `TurnContextResolver`
effect; core itself never executes a phase or receives provider configuration
or stored records.

Environment implementations live in `everruns-integrations-filesystem`,
`everruns-integrations-bashkit`, `everruns-integrations-web-fetch`,
`everruns-integrations-lua`, `everruns-mcp`, and `everruns-http`.
Application-grade in-memory backends live in `everruns-host`; deterministic
writable fixtures live in `everruns-test-support`, while the production-safe
offline simulator driver lives in `everruns-llmsim`.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-core)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [The agentic loop](https://docs.everruns.com/explanation/agentic-loop/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
