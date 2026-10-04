# everruns-core

> Core agent abstractions for the Everruns durable agentic harness engine.

[![Crates.io](https://img.shields.io/crates/v/everruns-core.svg)](https://crates.io/crates/everruns-core)
[![Documentation](https://docs.rs/everruns-core/badge.svg)](https://docs.rs/everruns-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-core` supplies portable execution contracts and optional runtime modules
for the [Everruns](https://everruns.com) ecosystem. Its default feature set is empty
and builds for `wasm32-unknown-unknown` without network, filesystem, process,
telemetry-exporter, or database dependencies. Control-plane records belong to the
server; concrete provider drivers belong to `everruns-drivers`.

## Quick Example

```rust
use everruns_core::CapabilityRegistry;
use everruns_contracts::DriverRegistry;

let capabilities = CapabilityRegistry::new();
let drivers = DriverRegistry::new();
assert!(capabilities.is_empty());
assert!(drivers.registered_providers().is_empty());
```

## Features

| Feature | Public module | Purpose |
| --- | --- | --- |
| `agent-package` | `agent_package` | Authored formats, ZIP, virtual folders and semantic diffs |
| `agent-package-fs` | `agent_package` | Native disk loading and export |
| `engine` | `engine` | Portable Input/Reason/Act algorithms and turn planning |
| `builtins` | `builtins` | First-party policies over injected execution contracts |
| `host` | `host` | Host composition, stores, filesystem and runtime orchestration |
| `mcp` | `mcp` | MCP protocol, authentication and injected-transport adaptation |
| `ag-ui` | `ag_ui` | AG-UI wire types and conformance validation |
| `ag-ui-projection` | `ag_ui` | Canonical runtime-event projection |
| `ag-ui-client` | `ag_ui::client` | Optional outbound AG-UI HTTP client |
| `a2a` | `a2a` | Optional outbound A2A discovery and client construction |

Host networking, processes, containment and exporters have separate opt-ins:
`direct-egress`, `process`, `native-containment`, `host-shell`,
`openai-agents-api`, `otel`, and `braintrust`. `mcp-stdio` enables local-process
MCP. `openapi`, `tree-sitter-outlines`, and `ui-capabilities` select their
respective schema, structural-outline, and UI catalog surfaces.

Custom hosts inject `host::HostComposition` and `host::HostBackends`.
Applications use `everruns`; its `batteries` module selects the default policy
catalog and enabled environment integrations. The core host never selects
concrete vendor drivers or depends on integration crates.

The deprecated `everruns-engine`, `everruns-host`, `everruns-builtins`,
`everruns-mcp`, and `everruns-ag-ui` crates forward to these modules for one
release. Migrate imports and features to `everruns-core` before their removal.

The Framework exposes portable definitions through `everruns::AgentPackage`. See the [file-based agent guide](https://docs.everruns.com/how-to/define-agents-as-files/).

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-core)
- [Core concepts and execution model](https://docs.everruns.com/getting-started/concepts/)
- [The agentic loop](https://docs.everruns.com/explanation/agentic-loop/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
