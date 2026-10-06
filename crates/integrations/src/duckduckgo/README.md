# everruns-integrations

This integration is shipped as the `duckduckgo` module of `everruns-integrations` Enable it with `features = ["duckduckgo"]` in Cargo.

> DuckDuckGo Instant Answer search for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` adds a small, no-key capability that lets
agents pull quick facts, abstracts, definitions, and related topics from the
DuckDuckGo Instant Answer API. It registers a single stateless
`duckduckgo_instant_answer` tool, a low-friction way to give an agent quick
fact lookups without configuring any credentials. This is an instant-answer
lookup, not a full web/SERP search: an empty result does not mean no matching
web pages exist.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-contracts`](https://crates.io/crates/everruns-contracts) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::duckduckgo::DuckDuckGoCapability;

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

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [DuckDuckGo integration](https://docs.everruns.com/integrations/duckduckgo/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
