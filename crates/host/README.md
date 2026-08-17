# everruns-host

> Shared effectful orchestration for Everruns execution hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-host.svg)](https://crates.io/crates/everruns-host)
[![Documentation](https://docs.rs/everruns-host/badge.svg)](https://docs.rs/everruns-host)
[![License](https://img.shields.io/crates/l/everruns-host.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

Shared effectful host orchestration used by the Everruns application facade,
local adapters, workers, and advanced custom hosts.

This crate is a published implementation boundary so those crates can share a
single execution path. It is not the ordinary application entrypoint; most
applications in the [Everruns](https://everruns.com) ecosystem should use
[`everruns`](https://crates.io/crates/everruns) and the
[Framework guide](https://docs.everruns.com/framework/).

The portable phase algorithms and pure, sans-I/O turn planner live in
[`everruns-engine`](https://crates.io/crates/everruns-engine). Host resolves
deployment services and composes those engine executors; it does not carry a
second Input/Reason/Act implementation.

## Quick Example

```rust
use everruns_host::{ResolvedTurnInputs, RuntimeHostAdapter};

fn accepts_host<A: RuntimeHostAdapter>() {}
fn accepts_inputs(_: ResolvedTurnInputs) {}
# let _ = accepts_inputs;
```

## What It Provides

- Canonical event append, bounded replay, and read-only history projection
- Backend/store composition and low-level in-process execution
- Reference `InMemoryAgentStore`, `InMemoryHarnessStore`,
  `InMemorySessionStore`, and `InMemoryProviderStore` implementations
- Store-backed snapshot/context loading, lifecycle validation, provider/driver
  resolution, and `StoreCommandHost` completion
- Shared input, reason, act, lifecycle, MCP, filesystem, and scheduling host work
- Session mutation/storage contracts and their portable `session` capabilities
- Policy-aware `DirectEgressService` behind the opt-in `direct-egress` feature
- OpenTelemetry and Braintrust exporters behind the opt-in `observability`
  feature, available as `everruns_host::observability`
- Host adapter contracts for worker, local, and advanced integrations
- Neutral extension ports for higher-level typed services, subagent delegates,
  and turn-dependent tools

## Integration features

`runtime_capability_registry()` is the Framework runtime preset owner. It
starts from an empty core registry, adds the runtime-safe portable catalog when
`builtins` is enabled, then adds only the compiled integrations.
`compose_runtime_capability_registry(registry)` applies that same feature-driven
composition to a caller-selected registry. `runtime_egress_service()`
returns the matching direct, policy-aware transport when a network-capable
integration is selected and a disabled service otherwise. `direct-egress`
exposes the transport explicitly; `builtins`, `filesystem`, `bashkit`,
`web-fetch`, `lua`, and `mcp` are independent host features;
`mcp-stdio` additionally enables local-process MCP servers. The
application-facing `everruns` crate selects the ordinary defaults.

Hosted product policy is layered above this crate. `everruns-platform`
implements these neutral extension ports; `everruns-host` has no dependency on
or feature for the platform crate.

`utility-openai` owns the concrete, environment-configured utility-model
client used by the server and worker. The provider-neutral `UtilityLlmService`
contract remains in core; embedders that supply their own implementation do
not need this feature or its HTTP/TLS dependency tree.

`everruns-engine` keeps turn planning sans I/O and owns portable phase execution
over injected contracts. Canonical events are the sole history write path: host
execution appends events, while `EventHistory` rebuilds messages from bounded,
sequence-ordered replay. No writable message store is part of the maintained
host API.

The execution boundary is value-first: host orchestration resolves stores and
credential-bearing provider configuration, then passes core a secret-free
`ResolvedExecutionSnapshot`, filtered messages, model/provider identity, and
an opaque ready driver. Persisted Agent, Harness, and Session records never
cross into kernel execution or public context inspection.

## Documentation

- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns-host)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
