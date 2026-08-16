# everruns-integrations-browserless

> Cloud browser automation for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-browserless.svg)](https://crates.io/crates/everruns-integrations-browserless)
[![Documentation](https://docs.rs/everruns-integrations-browserless/badge.svg)](https://docs.rs/everruns-integrations-browserless)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-browserless` gives agents a headless cloud browser through
the [Browserless](https://www.browserless.io) REST API and CDP (Chrome DevTools
Protocol) WebSocket sessions. Agents can capture screenshots, read the DOM, scrape
structured data, and drive multi-step browser flows.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_browserless::BrowserlessCapability;

let capability = BrowserlessCapability;

assert_eq!(capability.id(), "browserless");
```

## What It Provides

- Screenshots and DOM reads of remote pages
- Structured scraping of page content
- Multi-step browser automation over CDP WebSocket sessions
- Bring-your-own Browserless API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-browserless)
- [Browserless integration](https://docs.everruns.com/integrations/browserless/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
