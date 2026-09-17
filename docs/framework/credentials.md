---
title: Credentials
description: Each driver declares the environment variables its own vendor SDK reads, and the Framework resolves them through one shared path.
---

Every provider driver declares the environment variables it reads, on its own
descriptor, following **its vendor's own SDK convention**. There is no Everruns
naming scheme to learn: if your shell already runs the `openai` CLI, the AWS
CLI, or an Azure service principal, it already configures the matching driver.

```rust
use everruns::{Agent, OpenAI};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Reads OPENAI_API_KEY, and OPENAI_BASE_URL when set.
let agent = Agent::builder()
    .instructions("Be concise.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;
# let _ = agent;
# Ok(())
# }
```

Each driver crate offers the same entry point, returning a ready `Provider`.
The facade bundles OpenAI behind its `openai` feature; other drivers are
separate crates you add as dependencies:

```rust
use everruns::{Agent, Model};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Reads ANTHROPIC_API_KEY, and ANTHROPIC_BASE_URL when set.
let model = Model::new("claude-sonnet-5", everruns_anthropic::from_env("anthropic")?);
# let _ = model;
# Ok(())
# }
```

## What each driver declares

| Driver | Credential | Endpoint |
| --- | --- | --- |
| OpenAI | `OPENAI_API_KEY` | `OPENAI_BASE_URL` |
| OpenAI (Chat Completions) | `OPENAI_API_KEY` | `OPENAI_BASE_URL` |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` | `AZURE_OPENAI_ENDPOINT` |
| Anthropic | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL` |
| Google Gemini | `GEMINI_API_KEY`, or `GOOGLE_API_KEY` | `GEMINI_BASE_URL` |
| OpenRouter | `OPENROUTER_API_KEY` | `OPENROUTER_BASE_URL` |
| Fireworks AI | `FIREWORKS_API_KEY` | `FIREWORKS_BASE_URL` |
| Meta Model API | `LLAMA_API_KEY`, or `META_API_KEY` | `LLAMA_BASE_URL` |
| AWS Bedrock | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION` (or `AWS_DEFAULT_REGION`), `AWS_SESSION_TOKEN` | — (the region selects it) |
| Microsoft MAI | `AZURE_AI_API_KEY`, **or** `AZURE_TENANT_ID` + `AZURE_CLIENT_ID` + `AZURE_CLIENT_SECRET` | `AZURE_AI_ENDPOINT` |

A driver is not limited to one key. Bedrock needs four AWS fields; MAI accepts
either a resource key or a full Entra ID service principal. Alternates listed
with "or" are variables the vendor itself also honors, tried in the order
shown — not a second credential.

A credential resolves whole or not at all. If any required variable is missing
the driver is simply not configured from the environment, rather than being
half-configured into a provider that fails at its first request. A shell
carrying `AWS_REGION` but no AWS keys does not configure Bedrock, and a
half-populated Entra block does not configure MAI — the same rule the schema
applies to an operator-entered form.

This table is pinned by a test against the drivers' own declarations, so it
cannot drift from what they read.

## Custom drivers

A [custom driver](/framework/custom-providers/) declares its variables the same
way, on the credential field itself:

```rust
use everruns::{CredentialFormSchema, DriverDescriptor, DriverId, FormField};

# fn descriptor_for(factory: fn(&everruns::DriverConfig) -> everruns::BoxedChatDriver) -> DriverDescriptor {
DriverDescriptor {
    display_name: "Acme".into(),
    credential_schema: CredentialFormSchema {
        fields: vec![
            FormField::password("api_key", "API Key")
                .required()
                .env("ACME_API_KEY"),
        ],
        instructions_markdown: "Create a key in the Acme console.".into(),
    },
    base_url_env: Some("ACME_BASE_URL".into()),
    ..DriverDescriptor::chat_only(DriverId::external("acme"), factory)
}
# }
```

A driver that declares nothing is never configured from the environment,
whatever its id is spelled. That is the safe default: the registry's built-in
schema declares no variable, so a driver opts in by naming what its vendor
reads.

## Server deployments never read the environment

Credential loading is an injected concern, not something a driver does. A
driver only *declares* names; the declaration reads nothing and is simply never
consulted on the server.

`EnvCredentialProvider` is the single place in the workspace that pairs a
driver's declarations with a real environment lookup, and it is for standalone,
CLI, and development use. The multitenant server resolves credentials from its
encrypted database and constructs no `CredentialProvider` at all. This is the
fail-closed Key Resolution Contract: a platform-level key reachable from a
shared host environment would silently fund tenant execution.

So `OpenAI::from_env`, each driver's `from_env`, and `EnvCredentialProvider`
belong in your own binaries and dev entrypoints. Hosted deployments configure
providers through the Settings UI, which renders the same declared schema.

## Resolving credentials yourself

To build the provider without going through a driver crate's `from_env` — a
custom `ProviderKey`, or your own credential source — resolve against the
descriptor:

```rust
use everruns::{CredentialProvider, EnvCredentialProvider, provider_from_env};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let driver = everruns_anthropic::descriptor();

// What this driver reads, for an error message or a setup check.
let names = driver.declared_env_vars();

// The same resolution `from_env` performs.
let provider = provider_from_env(&driver, "primary")?;

// Or inspect the resolved fields first.
if let Some(credentials) = EnvCredentialProvider.resolve(&driver) {
    let _ = credentials.api_key();
}
# let _ = (names, provider);
# Ok(())
# }
```

`ProviderCredentials` carries every declared field, so multi-field drivers stay
expressible; `document()` produces the exact credential shape the server
stores, which is why an env-resolved credential and an operator-entered one
reach the driver through one path.
