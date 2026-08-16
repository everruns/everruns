# everruns-integrations-duckduckgo

> DuckDuckGo Instant Answer search for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-duckduckgo.svg)](https://crates.io/crates/everruns-integrations-duckduckgo)
[![Documentation](https://docs.rs/everruns-integrations-duckduckgo/badge.svg)](https://docs.rs/everruns-integrations-duckduckgo)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-duckduckgo` adds a small, no-key capability that lets
agents pull quick facts, abstracts, definitions, and related topics from the
DuckDuckGo Instant Answer API. It registers a single stateless
`duckduckgo_instant_answer` tool, a low-friction way to give an agent quick
fact lookups without configuring any credentials. This is an instant-answer
lookup, not a full web/SERP search: an empty result does not mean no matching
web pages exist.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-core`](https://crates.io/crates/everruns-core) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_duckduckgo::DuckDuckGoCapability;

let capability = DuckDuckGoCapability;

assert_eq!(capability.id(), "duckduckgo");
assert_eq!(capability.tools().len(), 1);
```

## What It Provides

- `duckduckgo_instant_answer` tool registration
- No API-key setup
- Stateless instant-answer lookup for agents (not full web/SERP search)
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-duckduckgo)
- [DuckDuckGo integration](https://docs.everruns.com/integrations/duckduckgo/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
