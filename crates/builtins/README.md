# everruns-builtins

> Portable policy capabilities for the Everruns Framework.

[![Crates.io](https://img.shields.io/crates/v/everruns-builtins.svg)](https://crates.io/crates/everruns-builtins)
[![Documentation](https://docs.rs/everruns-builtins/badge.svg)](https://docs.rs/everruns-builtins)
[![License](https://img.shields.io/crates/l/everruns-builtins.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This crate owns the backend-neutral implementations that shape agent behavior,
including context compaction, tool search, budgeting, loop and progress guards,
prompt caching, tool-call repair, output handling, and guardrails. Linking the
crate does not register anything: applications choose a registry and call
`register_portable_capabilities` explicitly.
Registration rejects ID or alias collisions atomically, leaving the caller's
registry unchanged on error.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## Quick Example

```rust
use everruns_builtins::register_portable_capabilities;
use everruns_core::CapabilityRegistry;

let mut registry = CapabilityRegistry::new();
register_portable_capabilities(&mut registry)?;

# Ok::<(), everruns_capability::CapabilityError>(())
```

The bundle contains policy, not environment integrations. It does not own a
network client, process runner, interpreter, database, server, or hosted
service. Capabilities that persist or distill tool output declare a dependency
on `session_file_system`; the embedding application must compose a compatible
filesystem implementation when enabling those capabilities.
`usage_limit_auto_continue` similarly requires the host's session-schedule
service; `register_runtime_capabilities` omits it for embedded runtimes that do
not provide that service.

Use the `everruns` facade for the normal Framework API. Depend on this crate
directly when building a custom capability registry or a minimal host.

## What It Provides

- Explicit runtime-safe and full-product registration functions
- Context compaction, tool search, budgeting, prompt caching, and guard policies
- Tool-call repair and output persistence/distillation policy hooks
- Public typed configuration values re-exported by the `everruns` facade
- Atomic capability-ID and alias collision rejection

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-builtins)
- [Framework advanced capabilities](https://docs.everruns.com/framework/advanced-capabilities/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
