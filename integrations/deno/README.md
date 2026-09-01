# everruns-integrations-deno

> Deno cloud sandboxes for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-deno.svg)](https://crates.io/crates/everruns-integrations-deno)
[![Documentation](https://docs.rs/everruns-integrations-deno/badge.svg)](https://docs.rs/everruns-integrations-deno)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-deno` adds cloud-based sandboxed code execution backed by
Deno Sandboxes, letting agents run code inside isolated environments without
touching the host. Sandboxes are managed per session, each identified by its own
sandbox id.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_deno::DenoCapability;

let capability = DenoCapability;

assert_eq!(capability.id(), "deno");
```

## What It Provides

- Per-session Deno sandbox lifecycle, with multiple sandboxes per session
- Sandboxed code execution inside an isolated environment
- Bring-your-own API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-deno)
- [Everruns documentation](https://docs.everruns.com)

> **Unsupported and untested.** Deno sandboxes require a paid Deno plan; without
> one, `create_sandbox` returns `400 VERIFICATION_REQUIRED_FOR_SANDBOXES`. This
> crate has no live coverage — only mock-backed unit tests — so it is not
> exercised against the real control plane and is not listed in the public
> documentation. `SPEC.md` has the details.

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
