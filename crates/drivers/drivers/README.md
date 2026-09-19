# everruns-drivers

> Everruns model drivers for vendors that need no bespoke wire protocol.

[![Crates.io](https://img.shields.io/crates/v/everruns-drivers.svg)](https://crates.io/crates/everruns-drivers)
[![Documentation](https://docs.rs/everruns-drivers/badge.svg)](https://docs.rs/everruns-drivers)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-drivers` registers vendor drivers into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider). Each module
wraps one of that crate's shared protocol drivers — `OpenAIProtocolChatDriver`
(Chat Completions) or `OpenResponsesProtocolChatDriver`
([Open Responses](https://openresponses.org)) — and adds only what is
vendor-specific: identity, credential schema, authentication, base URL, and
model discovery.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Providers are swappable: see
[`everruns-openai`](https://crates.io/crates/everruns-openai) for OpenAI models,
or [`everruns-anthropic`](https://crates.io/crates/everruns-anthropic) for Claude
models.

| Feature | Driver | `DriverId` | Wire protocol |
| --- | --- | --- | --- |
| `cloudflare` | Cloudflare AI Gateway | `cloudflare` | OpenAI Chat Completions (`/compat`) |
| `vercel` | Vercel AI Gateway | `vercel` | Open Responses |

No vendor is enabled by default, so a consumer compiles and ships only the ones
it serves:

```toml
everruns-drivers = { version = "0.29.0", features = ["cloudflare", "vercel"] }
```

## Driver-Only Example

```rust
use everruns_drivers::{DriverRegistry, register_drivers};

let mut registry = DriverRegistry::new();
register_drivers(&mut registry);
```

## What It Provides

- A Cloudflare AI Gateway driver over the gateway's OpenAI-compatible `/compat`
  endpoint, deriving its URL from the account and gateway ids and
  authenticating with `cf-aig-authorization`
- A Vercel AI Gateway driver over the gateway's Open Responses API, with model
  discovery gated to the gateway host
- Registration into the Everruns `DriverRegistry`, per vendor or all at once
  via `register_drivers`
- `base_url` override on every driver, for a proxy in front of the vendor

## Why One Crate

A vendor whose API is already OpenAI-compatible needs a descriptor, a
credential schema, auth, and a base URL — not a wire implementation. A separate
package for each would cost a workspace member, an entry in four isolation
guards, a CI shard, and a publish pin every time; a module costs none of that.

A vendor graduates to its own crate in two cases, both about what the rest of
the workspace pays. **Its own wire**: a vendor needing its own request and
response types is a protocol implementation, not a binding
([`everruns-anthropic`](https://crates.io/crates/everruns-anthropic),
[`everruns-gemini`](https://crates.io/crates/everruns-gemini)). **Heavy
dependencies**: this crate is a single dependency edge, so a dependency added
for one vendor lands on every consumer of the others
([`everruns-bedrock`](https://crates.io/crates/everruns-bedrock) and the AWS
SDK).

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-drivers)
- [Cloudflare AI Gateway provider guide](https://docs.everruns.com/providers/cloudflare/)
- [Vercel AI Gateway provider guide](https://docs.everruns.com/providers/vercel/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
