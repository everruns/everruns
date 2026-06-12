# everruns-integrations-brave-search

> Brave Search web search for Everruns agents.

`everruns-integrations-brave-search` lets agents search the web through the
[Brave Search](https://brave.com/search/api/) REST API. It registers a single
`brave_web_search` tool, authenticated with a user-supplied Brave Search API key.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_brave_search::BraveSearchCapability;

let capability = BraveSearchCapability;

assert_eq!(capability.id(), "brave_search");
```

## What It Provides

- `brave_web_search` tool registration
- Web search over the Brave Search REST API
- Bring-your-own Brave Search API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [Brave Search integration](https://docs.everruns.com/integrations/brave-search/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
