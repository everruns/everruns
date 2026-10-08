# everruns-drivers

> Everruns model drivers, one feature per vendor.

[![Crates.io](https://img.shields.io/crates/v/everruns-drivers.svg)](https://crates.io/crates/everruns-drivers)
[![Documentation](https://docs.rs/everruns-drivers/badge.svg)](https://docs.rs/everruns-drivers)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-drivers` registers vendor drivers into a `DriverRegistry` from
[`everruns-contracts`](https://crates.io/crates/everruns-contracts). Vendors whose
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
| `meta` | `meta` | Meta | Open Responses |
| `mistral` | `mistral` | Mistral AI (La Plateforme) | OpenAI Chat Completions |
| `openai` | `openai` | OpenAI and Azure OpenAI | Responses and Chat Completions |
| `openrouter` | `openrouter` | OpenRouter | OpenAI Responses-compatible |
| `typesafe` | `typesafe` | TypeSafe (typed decisions) | System One |
| `vercel` | `vercel` | Vercel AI Gateway | Open Responses |

No vendor is enabled by default, so a consumer compiles and ships only the ones
it serves. Vendor dependencies such as the AWS SDK are optional and come in
only with their feature:

```toml
everruns-drivers = { version = "0.41", features = ["openai", "anthropic"] }
```

## Quick start: get a reply

Add these dependencies to your application's `Cargo.toml`:

```toml
[dependencies]
everruns-drivers = { version = "0.41", features = ["openai"] }
everruns-contracts = { version = "0.41", default-features = false }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Set `OPENAI_API_KEY` in the environment before running this standalone application.
Replace `your-model-id` with a model available to your account.

```no_run
# #[cfg(feature = "openai")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use everruns_contracts::{LlmCallConfig, Message, MessageRole};
    use everruns_drivers::openai;

    let provider = openai::from_env("openai")?;
    let config = LlmCallConfig::new("your-model-id");
    let response = provider
        .chat_completion(
            vec![Message::text(MessageRole::User, "Hello!")],
            &config,
        )
        .await?;

    println!("{}", response.text);
    println!("Token usage: {:?}", response.metadata.total_tokens);
    Ok(())
}
# #[cfg(not(feature = "openai"))]
# fn main() {}
```

`chat_completion` collects the provider stream into a full response. The response
also carries tool calls, reasoning artifacts, and completion metadata. Drivers
perform one model call; your application or the Everruns runtime owns the tool loop.

For Anthropic, enable `anthropic`, use `anthropic::from_env("anthropic")`, and set
`ANTHROPIC_API_KEY`. The message and call configuration types stay the same.

### Credentials and endpoints

A runtime `Provider` binds a driver to its endpoint and authentication.
For a key supplied by your application's configuration, construct it explicitly:

```rust
# #[cfg(feature = "openai")]
# {
use everruns_drivers::openai;

let provider = openai::provider("my-openai", "your-api-key")
    .base_url("https://your-proxy.example/v1");
assert_eq!(provider.id().as_str(), "my-openai");
# }
```

The provider ID is your connection's identity; `LlmCallConfig::new` takes the
vendor's model ID. `from_env` is an explicit standalone/CLI convenience that
resolves the vendor's declared environment variables, including `OPENAI_BASE_URL`
for OpenAI. Hosts serving multiple tenants should supply each tenant's credentials
explicitly. Personal ChatGPT and Codex connections use host-owned authentication;
see [Personal ChatGPT authentication](#personal-chatgpt-authentication).

## Streaming

Add `futures = "0.3"` to the quick-start dependencies to consume incremental events:

```no_run
# #[cfg(feature = "openai")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use everruns_contracts::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
    use everruns_drivers::openai;
    use futures::StreamExt;
    use std::io::{self, Write};

    let provider = openai::from_env("openai")?;
    let mut stream = provider
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "Hello!")],
            &LlmCallConfig::new("your-model-id"),
        )
        .await?;

    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event? {
            LlmStreamEvent::TextDelta(delta) => {
                print!("{delta}");
                io::stdout().flush()?;
            }
            LlmStreamEvent::Done(metadata) => {
                completed = true;
                eprintln!("\nToken usage: {:?}", metadata.total_tokens);
            }
            LlmStreamEvent::Error(error) => return Err(error.into()),
            _ => {}
        }
    }
    if !completed {
        return Err("stream ended without a completion event".into());
    }
    println!();
    Ok(())
}
# #[cfg(not(feature = "openai"))]
# fn main() {}
```

Stream items can fail with a transport error, and providers can also emit an
`Error` event. Handle both. `LlmStreamEvent` is non-exhaustive, so keep a wildcard
arm; tool calls and other events need handling when building an agent loop.

## Register enabled drivers

Hosts that select providers from configuration can register the enabled vendors
once, then create a credential-bearing provider from the registry:

```rust
# #[cfg(feature = "openai")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use everruns_contracts::{DriverId, ProviderConfig};
use everruns_drivers::{DriverRegistry, register_drivers};

let mut registry = DriverRegistry::new();
register_drivers(&mut registry);

let config = ProviderConfig::new(DriverId::OpenAI)
    .with_api_key("your-api-key");
let provider = registry.create_provider(&config)?;
assert_eq!(provider.id().as_str(), "openai");
# Ok(())
# }
# #[cfg(not(feature = "openai"))]
# fn main() {}
```

Call the returned provider's `chat_completion` or `chat_completion_stream` as
above. `register_drivers` registers every enabled vendor; each module also offers
`register_driver` and `descriptor`. Registering drivers alone does not configure
credentials or contact a vendor. A disabled vendor is absent from the registry.

## Runnable examples

From a checkout of the Everruns repository, set the matching API key and run a
command below. Pass a model ID available to your account; an optional second
argument supplies the prompt.

| Example | Command | Credentials |
| --- | --- | --- |
| [OpenAI streaming](https://github.com/everruns/everruns/blob/main/crates/drivers/drivers/examples/openai-chat.rs) | `cargo run -p everruns-drivers --features openai --example openai-chat -- <model-id> "Hello!"` | `OPENAI_API_KEY` |
| [Anthropic completion](https://github.com/everruns/everruns/blob/main/crates/drivers/drivers/examples/anthropic-chat.rs) | `cargo run -p everruns-drivers --features anthropic --example anthropic-chat -- <model-id> "Hello!"` | `ANTHROPIC_API_KEY` |
| [Registry selection](https://github.com/everruns/everruns/blob/main/crates/drivers/drivers/examples/registry-chat.rs) | `cargo run -p everruns-drivers --features openai --example registry-chat -- <model-id> "Hello!"` | `OPENAI_API_KEY` |

## Rust API documentation

[docs.rs](https://docs.rs/everruns-drivers) builds with all vendor features enabled,
so every provider module and its public API appears in the reference. Your
application still compiles only the features it enables. To generate the same
reference locally:

```sh
cargo doc -p everruns-drivers --all-features --no-deps --open
```

Start with the module for your vendor (for example,
[`openai`](https://docs.rs/everruns-drivers/latest/everruns_drivers/openai/index.html)
or [`anthropic`](https://docs.rs/everruns-drivers/latest/everruns_drivers/anthropic/index.html)).
Shared request, response, and registry types live in
[`everruns-contracts`](https://docs.rs/everruns-contracts).

## What It Provides

- A `ChatDriver` per chat vendor, each behind its own feature, plus typed services such as TypeSafe decisions
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
everruns-drivers = { version = "0.41", features = ["openai"] }
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
