# everruns-mai

Microsoft MAI provider driver for [Everruns](https://everruns.com).

Microsoft MAI models (e.g. `MAI-Code-1-Flash`) are served via
[Azure AI Foundry](https://ai.azure.com) behind an OpenAI-compatible Chat
Completions API. `everruns-mai` implements the `ChatDriver` contract from
[`everruns-core`](https://crates.io/crates/everruns-core) and registers the
`mai` driver into a `DriverRegistry`.

## Authentication

Two schemes are supported:

- **Azure AI Foundry API key** — the resource key, sent via the `api-key`
  header.
- **Microsoft Entra ID (OAuth)** — a client-credentials service principal
  (`tenant_id`, `client_id`, `client_secret`), supplied through provider
  metadata. Bearer tokens are minted with the client-credentials grant and
  cached, refreshed before expiry.

The auth layer is built on the pluggable `AuthHeaderProvider` hook in
`everruns-core`, so additional schemes (managed identity, workload identity
federation, ...) can be added by implementing the trait without changing the
driver.

## Usage

```rust
use everruns_core::DriverRegistry;
use everruns_mai::{register_driver, MaiAuth, MaiChatDriver};

// Register into a driver registry (the usual integration path):
let mut registry = DriverRegistry::new();
register_driver(&mut registry);

// Or construct a driver directly:
let driver = MaiChatDriver::new(
    MaiAuth::ApiKey("foundry-key".into()),
    "https://my-resource.services.ai.azure.com",
);
```

## License

MIT
