# everruns-integrations-duckduckgo

DuckDuckGo Instant Answer search integration for Everruns agents.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It adds a
small no-key network capability for quick facts, abstracts, definitions, and
related topics from the DuckDuckGo Instant Answer API.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_duckduckgo::DuckDuckGoCapability;

let capability = DuckDuckGoCapability;

assert_eq!(capability.id(), "duckduckgo");
assert_eq!(capability.tools().len(), 1);
```

## What It Provides

- `duckduckgo_search` tool registration
- No API-key setup
- Stateless instant-answer lookup for agents
- Inventory-based Everruns integration registration

## License

MIT. See the repository-level `LICENSE` file.
