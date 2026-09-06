# everruns-integrations-brave-search

> Brave Search web search for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-brave-search.svg)](https://crates.io/crates/everruns-integrations-brave-search)
[![Documentation](https://docs.rs/everruns-integrations-brave-search/badge.svg)](https://docs.rs/everruns-integrations-brave-search)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-brave-search` lets agents search the web through the
[Brave Search](https://brave.com/search/api/) REST API. It registers a single
`brave_web_search` tool, authenticated with a user-supplied Brave Search API key.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns::{Agent, Model};
use everruns_integrations_brave_search::BraveSearch;

let agent = Agent::builder()
    .instructions("Search the web and cite source URLs.")
    .model(Model::simulated("Ready."))
    .capability(BraveSearch::from_env()?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Framework applications select `default-features = false` on this integration
dependency and enable the `everruns` `capabilities` feature (enabled by default).
Set `BRAVE_SEARCH_API_KEY` before startup, or pass an application-owned key to
`BraveSearch::new`. Credentials stay inside the client, outside agent config and
metadata. `BraveSearch::with_client` accepts a client with a trusted custom
endpoint for HTTP tests. Run the agent with `Engine::new().create(agent)`.

The default `hosted` feature includes connector UI metadata and inventory
registration for the Platform. Hosted tools retain lazy connection-token lookup
and session-secret fallback. Both adapters share the `brave_web_search` schema,
search operation, and result mapping. Framework credentials are read at
construction; rebuild the capability to rotate them. Framework calls use the
application-owned HTTP client, not hosted egress or connection policies.

## What It Provides

- `brave_web_search` tool registration
- Web search over the Brave Search REST API
- Bring-your-own Brave Search API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-brave-search)
- [Brave Search integration](https://docs.everruns.com/integrations/brave-search/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
