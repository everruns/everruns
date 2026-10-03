# everruns-drivers

> Everruns model drivers, one feature per vendor.

[![Crates.io](https://img.shields.io/crates/v/everruns-drivers.svg)](https://crates.io/crates/everruns-drivers)
[![Documentation](https://docs.rs/everruns-drivers/badge.svg)](https://docs.rs/everruns-drivers)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-drivers` registers vendor drivers into a `DriverRegistry` from
[`everruns-provider`](https://crates.io/crates/everruns-provider). Vendors whose
API is OpenAI-compatible wrap one of that crate's shared protocol drivers,
`OpenAIProtocolChatDriver` (Chat Completions) or
`OpenResponsesProtocolChatDriver` ([Open Responses](https://openresponses.org)),
and add only identity, credential schema, authentication, base URL, and model
discovery. Anthropic, Gemini, and Bedrock carry their own wire types.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. Most applications reach these
drivers through the [`everruns`](https://crates.io/crates/everruns) facade, whose
vendor features turn on the matching features here and re-export this crate as
`everruns::drivers`.

| Feature | Module | Driver | Wire protocol |
| --- | --- | --- | --- |
| `anthropic` | `anthropic` | Anthropic Claude | Anthropic Messages |
| `bedrock` | `bedrock` | AWS Bedrock | Bedrock Converse |
| `bedrock-default-credentials` | `bedrock` | AWS default credential chain | |
| `chatgpt` | `chatgpt` | Personal ChatGPT plan | Stateless Responses + open-source OAuth |
| `codex` | `codex` | Legacy Codex | Codex Responses |
| `cloudflare` | `cloudflare` | Cloudflare AI Gateway | OpenAI Chat Completions |
| `fireworks` | `fireworks` | Fireworks AI | OpenAI Chat Completions |
| `gemini` | `gemini` | Google Gemini | Gemini API |
| `mai` | `mai` | Microsoft AI (Foundry) | OpenAI Chat Completions |
| `meta` | `meta` | Meta Model API | Open Responses |
| `openai` | `openai` | OpenAI and Azure OpenAI | Responses and Chat Completions |
| `openrouter` | `openrouter` | OpenRouter | OpenAI Responses-compatible |
| `vercel` | `vercel` | Vercel AI Gateway | Open Responses |

No vendor is enabled by default, so a consumer compiles and ships only the ones
it serves. Vendor dependencies such as the AWS SDK are optional and come in
only with their feature:

```toml
everruns-drivers = { version = "0.35", features = ["openai", "anthropic"] }
```

## Driver-Only Example

```rust
use everruns_drivers::{DriverRegistry, register_drivers};

let mut registry = DriverRegistry::new();
register_drivers(&mut registry);
```

`register_drivers` registers every enabled vendor. Each module also has its own
`register_driver`, `descriptor`, and `from_env`.

## What It Provides

- A `ChatDriver` per vendor, each behind its own feature
- Registration into the Everruns `DriverRegistry`, per vendor or all enabled
  vendors at once via `register_drivers`
- Credential schemas and `from_env` constructors that read each vendor's own
  environment variables, for standalone and CLI use
- Host-gated model discovery and a `base_url` override on every driver, for a
  proxy in front of the vendor

## Moved Crates

`everruns-anthropic`, `everruns-bedrock`, `everruns-fireworks`,
`everruns-gemini`, `everruns-mai`, `everruns-meta`, `everruns-openai`, and
`everruns-openrouter` are now modules of this crate. Their final release, 0.35, is a
deprecated shim that re-exports the module. The shim packages are retired from
the workspace and publish set; all vendors remain available here.
To migrate, replace the dependency with this crate and the vendor's feature:

```toml
# before
everruns-openai = "0.34"
# after
everruns-drivers = { version = "0.35", features = ["openai"] }
```

and `everruns_openai::X` with `everruns_drivers::openai::X`. Bedrock's
`default-credentials` feature is `bedrock-default-credentials` here.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-drivers)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Migrate between LLM providers](https://docs.everruns.com/how-to/migrate-providers/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

## Personal ChatGPT authentication

Enable `chatgpt` for the public plan route or `codex` for the legacy backend.
Hosts provide the browser opener and an implementation of
`chatgpt::auth::TokenStore`; `RotatingAuth` loads credentials under its lease and
persists the full rotated pair before producing authentication headers. Hosts
with writers outside that lease must implement atomic `compare_and_save`.
`chatgpt::login::LoginAttempt` handles loopback PKCE and verified dynamic
registration; credentials and registration are host-owned.

The `chatgpt-login` example provides a private-file handoff for remote self-hosted
installations. See [ChatGPT plan](https://docs.everruns.com/features/chatgpt/).
