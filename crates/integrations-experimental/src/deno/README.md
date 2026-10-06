# everruns-integrations-experimental

This integration is shipped as the `deno` module of `everruns-integrations-experimental`. Enable it with `features = ["deno"]` in Cargo.

> Deno cloud sandboxes for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-experimental` adds cloud-based sandboxed code execution backed by
Deno Sandboxes, letting agents run code inside isolated environments without
touching the host. Sandboxes are managed per session, each identified by its own
sandbox id.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-contracts`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations_experimental::deno::DenoCapability;

let capability = DenoCapability;

assert_eq!(capability.id(), "deno");
```

## What It Provides

- Per-session Deno sandbox lifecycle, with multiple sandboxes per session
- Sandboxed code execution inside an isolated environment
- Bring-your-own API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-experimental)
- [Everruns documentation](https://docs.everruns.com)

> **Unsupported and untested.** Deno sandboxes require a paid Deno plan; without
> one, `create_sandbox` returns `400 VERIFICATION_REQUIRED_FOR_SANDBOXES`. This
> crate has no live coverage — only mock-backed unit tests — so it is not
> exercised against the real control plane and is not listed in the public
> documentation. `SPEC.md` has the details.

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
