# everruns-integrations-web-fetch

> Policy-routed web fetching for Everruns agents.

`everruns-integrations-web-fetch` adapts FetchKit's schema, extraction,
download, signing, and SSRF controls to Everruns capability and egress
contracts.

Part of the [Everruns](https://everruns.com) ecosystem. Framework applications
enable it with the `web-fetch` feature; hosted product composition registers it
explicitly.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_web_fetch::WebFetchCapability;

assert_eq!(WebFetchCapability::new(None).id(), "web_fetch");
```

## What It Provides

- FetchKit-backed `web_fetch` tool and delegated schema
- DNS-pinned SSRF and host egress-policy integration
- Optional session-file downloads
- Cancellation-safe, bounded response handling and request signing

## Documentation

- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [API reference](https://docs.rs/everruns-integrations-web-fetch)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
