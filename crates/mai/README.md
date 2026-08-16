# everruns-mai

Microsoft MAI provider driver for [Everruns](https://everruns.com).

Microsoft MAI models (e.g. `MAI-Code-1-Flash`) are served via
[Azure AI Foundry](https://ai.azure.com) behind an OpenAI-compatible Chat
Completions API. `everruns-mai` implements the `ChatDriver` contract from
[`everruns-provider`](https://crates.io/crates/everruns-provider) and registers the
`mai` driver into a `DriverRegistry`.

## Authentication

Two schemes are supported:

- **Azure AI Foundry API key**: the resource key, sent via the `api-key`
  header.
- **Microsoft Entra ID (OAuth)**: a client-credentials service principal
  (`tenant_id`, `client_id`, `client_secret`), supplied through provider
  metadata. Bearer tokens are minted with the client-credentials grant and
  cached, refreshed before expiry.

The provider owns authentication through the pluggable `ProviderAuth` contract
in `everruns-provider`. Additional schemes (managed identity, workload identity
federation, ...) can be added without changing the Chat Completions wire driver.

## Usage

```rust
use everruns_mai::{provider, MaiAuth};

let provider = provider(
    "mai-prod",
    "https://my-resource.services.ai.azure.com/openai/v1",
    MaiAuth::ApiKey("foundry-key".into()),
);
```

## Documentation

- [Microsoft MAI provider guide](https://docs.everruns.com/providers/mai/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

MIT
