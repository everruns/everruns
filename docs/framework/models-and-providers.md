---
title: Models and Providers
description: Select model identities, attach provider drivers and credentials, list provider catalogs, and write a custom provider.
---

The Framework separates **what model to use** from **how to reach it**:

- `.model("id")` selects the provider-visible model with a credential-free string.
- `Provider` supplies the driver, endpoint, and authentication needed by the host.
- An agent currently accepts one provider, configured separately with `.provider(...)`.

This boundary is open: a new provider does not require a new closed enum variant
or provider-specific branch in application code. The Framework constructs its
execution-facing model specification internally when the agent builds.

This page covers the provider types the `everruns` crate bundles, the drivers
that ship today, the environment variables each driver reads, provider model
catalogs, and writing your own driver. To call a model once without building an
agent, see [Direct model calls](/framework/direct-model-calls/).

## OpenAI convenience

With the `openai` feature, `OpenAI::from_env` reads `OPENAI_API_KEY` and the
optional `OPENAI_BASE_URL`:

```rust
use everruns::{Agent, OpenAI};

let agent = Agent::builder()
    .instructions("Be concise.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `OpenAI::new(key)` when the host already owns an explicitly resolved
credential. Never put credentials in a model id, log them as model identity, or
select provider behavior with vendor-specific detection.

The `anthropic` and `gemini` features add `Anthropic` and `Gemini` with the
same shape, and `openrouter` adds `OpenRouter`:

| Feature | Type | Reads |
| --- | --- | --- |
| `openai` | `OpenAI` | `OPENAI_API_KEY`, `OPENAI_BASE_URL` |
| `anthropic` | `Anthropic` | `ANTHROPIC_API_KEY` |
| `gemini` | `Gemini` | `GEMINI_API_KEY` or `GOOGLE_API_KEY`, `GEMINI_BASE_URL` |
| `openrouter` | `OpenRouter` | `OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL` |

The `bedrock` feature adds `Bedrock`, which takes either static keys or the AWS
default credential chain, so a process with an IAM role (an ECS task, an EC2
instance profile, an Amazon Bedrock AgentCore Runtime execution role) needs no
keys. The model id is a Bedrock model id or inference profile:

```rust
use everruns::{Model, providers::bedrock::Bedrock};

// Static keys.
let model = Model::new(
    "us.anthropic.claude-sonnet-4-6",
    Bedrock::new("AKIA...", "secret", "us-east-1"),
);

// The AWS default chain. Region: `.region(..)`, else AWS_REGION, else
// AWS_DEFAULT_REGION, else us-east-1.
let model = Model::new(
    "us.anthropic.claude-sonnet-4-6",
    Bedrock::default_chain().region("us-east-1"),
);
```

`from_env` is not OpenAI-specific: every driver declares the variables its own
vendor SDK reads, and each driver crate exposes the same entry point. See
[Credentials](#credentials) for the per-driver table.

Applications with their own driver pass `Provider::new(key, driver)` to
`.provider(...)`; see [Custom providers](#custom-providers).

## Simulated models, for tests

```rust
use everruns::{Agent, Model};

let agent = Agent::builder()
    .instructions("Answer deterministically.")
    .model(Model::simulated("fixed response"))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

`Model::simulated` is backed by the focused `everruns-llmsim` crate. It is a
**test double that runs no inference** — it replays canned responses so tests
can assert on agent behavior without a network call or an API key. It is not a
local model and not a way to run Everruns without a provider. Depend on the
crate directly when building a low-level host or scripting multi-turn provider
behavior; ordinary Framework applications need only `everruns`.

It registers as a driver, but it reaches no network. For real work, pick a
provider from [Supported providers](#supported-providers).

## Supported providers

Everruns talks to model vendors through **drivers**. A driver owns one vendor's
wire protocol; a `Provider` pairs a driver with an endpoint and a credential.
The set below is what ships today. The boundary is open, so a
[custom driver](#custom-providers) is a first-class peer of these.

### Drivers

| Driver | Crate | Wire protocol | Services | Model discovery |
| --- | --- | --- | --- | --- |
| OpenAI | `everruns-drivers` (`openai`) | OpenAI Responses | chat, embeddings, realtime | yes |
| ChatGPT plan | `everruns-drivers` (`chatgpt`) | Stateless Responses + open-source OAuth | chat | account-visible models |
| OpenAI (Chat Completions) | `everruns-drivers` (`openai`) | OpenAI Chat Completions | chat | yes |
| Azure OpenAI | `everruns-drivers` (`openai`) | OpenAI Responses | chat | yes |
| Anthropic | `everruns-drivers` (`anthropic`) | Anthropic Messages | chat | yes |
| Google Gemini | `everruns-drivers` (`gemini`) | Gemini `generateContent` | chat | yes |
| AWS Bedrock | `everruns-drivers` (`bedrock`) | Bedrock `ConverseStream` (SigV4) | chat | no |
| OpenRouter | `everruns-drivers` (`openrouter`) | OpenAI Responses-compatible | chat | yes |
| Microsoft MAI | `everruns-drivers` (`mai`) | OpenAI Chat Completions (Azure AI Foundry) | chat | yes |
| Fireworks AI | `everruns-drivers` (`fireworks`) | OpenAI Chat Completions-compatible | chat | yes |
| Meta Model API | `everruns-drivers` (`meta`) | OpenAI Responses-compatible | chat | yes |
| Cloudflare AI Gateway | `everruns-drivers` (`cloudflare`) | OpenAI Chat Completions-compatible | chat | Workers AI only |
| Vercel AI Gateway | `everruns-drivers` (`vercel`) | Open Responses | chat | yes |
| LLM Simulator | `everruns-llmsim` | none — in-process test double | chat | no |

Every chat driver produces an incremental stream — server-sent events for the
HTTP protocols, `ConverseStream` for Bedrock — so token-by-token output works
everywhere, not just on one vendor. Tool calling and multi-turn tool results
work across all of them: each driver normalizes its vendor's shape into the same
typed events, which is why swapping a provider does not change application
code.

The environment variables each API driver reads are in [Credentials](#credentials).
ChatGPT plan connections use host-owned OAuth credentials; see [ChatGPT plan](/features/chatgpt/).
The optional `codex` SDK feature provides the legacy Codex backend for hosts such
as Yolop. Everruns Platform registers the public ChatGPT plan route.

### Beyond chat

Most drivers implement chat only. Two capabilities go further, and both are
OpenAI-only today:

**Embeddings.** The OpenAI driver powers embedding models for knowledge-base
retrieval alongside its chat models.

**Realtime voice (WebRTC + WebSocket).** A realtime voice session is negotiated
by the platform server, not the Framework. The browser posts its SDP offer to
`POST /v1/sessions/{session_id}/voice/calls` and the server answers it; a
separate route mints a short-lived client secret from the vendor. The
organization's own API key is used only server-side and never reaches the
browser. The server then opens a WebSocket sideband
(`wss://…/realtime?call_id=…`) to drive the call and collect transcripts, which
land in the session as ordinary events — so a voice turn and a typed turn are
the same session, readable through the same history and event streams. This
needs the platform server; an embedded Framework process does not expose it.

### Interactive connect

OpenRouter declares an OAuth connect flow, so an operator can choose "Connect
with OpenRouter" instead of pasting a key. Every other driver takes a credential
directly, entered in Settings or supplied in code.

### Choosing one

Any OpenAI-compatible gateway that speaks Responses or Chat Completions can
usually be reached by pointing the matching driver's `base_url` at it, rather
than writing a driver. Write a [custom driver](#custom-providers)
when the vendor's protocol genuinely differs, or when it needs authentication
that a bearer token cannot express.

## Credentials

Every provider driver declares the environment variables it reads, on its own
descriptor, following **its vendor's own SDK convention**. There is no Everruns
naming scheme to learn: if your shell already runs the `openai` CLI, the AWS
CLI, or an Azure service principal, it already configures the matching driver. `OpenAI::from_env()` in the
example above reads `OPENAI_API_KEY`, and `OPENAI_BASE_URL` when set.

Each driver crate offers the same entry point, returning a ready `Provider`.
The facade bundles OpenAI, Anthropic, Gemini and OpenRouter behind the
`openai`, `anthropic`, `gemini` and `openrouter` features (`Anthropic::from_env`
and friends), and AWS Bedrock behind `bedrock`, where
`Bedrock::default_chain()` resolves credentials the way the AWS SDK does (an
IAM role, SSO or a profile, or the `AWS_*` variables) instead of reading
declared names; any other driver is a separate crate you add as a dependency:

```rust
use everruns::{Agent, Model};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Reads FIREWORKS_API_KEY, and FIREWORKS_BASE_URL when set.
let model = Model::new(
    "accounts/fireworks/models/llama-v3p1-70b-instruct",
    everruns_drivers::fireworks::from_env("fireworks")?,
);
# let _ = model;
# Ok(())
# }
```

### What each driver declares

| Driver | Credential | Endpoint |
| --- | --- | --- |
| OpenAI | `OPENAI_API_KEY` | `OPENAI_BASE_URL` |
| OpenAI (Chat Completions) | `OPENAI_API_KEY` | `OPENAI_BASE_URL` |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` | — (see below) |
| Anthropic | `ANTHROPIC_API_KEY` | — (see below) |
| Google Gemini | `GEMINI_API_KEY`, or `GOOGLE_API_KEY` | `GEMINI_BASE_URL` |
| OpenRouter | `OPENROUTER_API_KEY` | `OPENROUTER_BASE_URL` |
| Fireworks AI | `FIREWORKS_API_KEY` | `FIREWORKS_BASE_URL` |
| Meta Model API | `LLAMA_API_KEY`, or `META_API_KEY` | `LLAMA_BASE_URL` |
| AWS Bedrock | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION` (or `AWS_DEFAULT_REGION`), `AWS_SESSION_TOKEN` | — (the region selects it) |
| Microsoft MAI | `AZURE_AI_API_KEY`, **or** `AZURE_TENANT_ID` + `AZURE_CLIENT_ID` + `AZURE_CLIENT_SECRET` | `AZURE_AI_ENDPOINT` |
| Cloudflare AI Gateway | `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_AI_GATEWAY_ID` | — (derived from the account) |
| Vercel AI Gateway | `AI_GATEWAY_API_KEY` | — (one fixed gateway host) |

A driver is not limited to one key. Bedrock needs four AWS fields; MAI accepts
either a resource key or a full Entra ID service principal. Alternates listed
with "or" are variables the vendor itself also honors, tried in the order
shown — not a second credential.

Anthropic and Azure OpenAI declare no endpoint variable on purpose. A
`base_url` here is the *versioned* API root — drivers append bare operation
paths to it, and the defaults end in `/v1` — whereas `ANTHROPIC_BASE_URL` and
`AZURE_OPENAI_ENDPOINT` name the bare host, because those SDKs add the version
segment themselves. Importing either verbatim would resolve to
`https://api.anthropic.com/messages` and fail. Point those drivers at a proxy
with an explicit `Provider::new(...).base_url(...)`, or through the Settings
UI, instead.

A credential resolves whole or not at all. If any required variable is missing
the driver is simply not configured from the environment, rather than being
half-configured into a provider that fails at its first request. A shell
carrying `AWS_REGION` but no AWS keys does not configure Bedrock, and a
half-populated Entra block does not configure MAI — the same rule the schema
applies to an operator-entered form.

This table is pinned by a test against the drivers' own declarations, so it
cannot drift from what they read.

### Custom drivers

A [custom driver](#custom-providers) declares its variables the same
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

### Server deployments never read the environment

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

### Resolving credentials yourself

To build the provider without going through a driver crate's `from_env` — a
custom `ProviderKey`, or your own credential source — resolve against the
descriptor:

```rust
use everruns::{CredentialProvider, EnvCredentialProvider, provider_from_env};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let driver = everruns_drivers::anthropic::descriptor();

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

## Model catalogs

Selecting a model needs an exact, provider-visible id. Anything that lets a
person choose one — a picker, a `--model` flag, a settings page — needs the
catalog behind it: which ids this provider serves, what they are called, and
what each one supports.

That is the same provider edge an [agent](/framework/agents/) or a
[direct call](/framework/direct-model-calls/) uses, asked a different question:

```rust
use everruns::{OpenAI, models};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
for model in models::list(OpenAI::from_env()?).await? {
    println!("{} — {}", model.id(), model.display_name().unwrap_or("?"));
}
# Ok(())
# }
```

Ids come back exactly as chat calls expect them, newest first, merged with the
model profile registry so a provider that returns bare ids still renders
human-readable names and descriptions.

`list` is a provider call: it costs a round trip and catalogs change rarely, so
cache the result instead of asking per keystroke.

### What each entry carries

`ModelInfo` separates the id from everything around it. The id is the
provider's own; the rest is display and capability metadata, absent when
neither the provider nor the registry knows it.

```rust
use everruns::ModelInfo;

fn describe(model: &ModelInfo) -> String {
    let name = model.display_name().unwrap_or(model.id());
    let window = model.context_window().unwrap_or_default();
    let tools = if model.supports_tools() { "tools" } else { "no tools" };
    format!("{name}: {window} tokens, {tools}")
}
```

- `id` — pass back unchanged.
- `display_name`, `description` — for rendering.
- `vendor` — who trained the model, which is not always who serves it: an
  aggregator offers many vendors' models.
- `context_window`, `supports_tools`, `supports_reasoning` — the common checks,
  from the profile registry.
- `profile` — the full `ModelProfile` behind those: limits, per-million-token
  prices, modalities, and capability flags.

Capability answers come from curated data, so `false` also covers "not in the
registry". Treat them as display hints rather than guarantees.

### From a selection to a run

A selection converts straight back into the `Model` the rest of the API takes,
bundled with the provider it was discovered through:

```rust
use everruns::{Agent, OpenAI, models};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let catalog = models::list(OpenAI::from_env()?).await?;
let picked = catalog
    .into_iter()
    .find(|model| model.supports_tools())
    .ok_or("no tool-calling model")?;

let agent = Agent::builder()
    .instructions("Be concise.")
    .model(picked.model())
    .build()?;
# let _ = agent;
# Ok(())
# }
```

No string handling, and nothing to reconfigure: the model already knows how to
be reached.

### Providers without a catalog

Not every provider can enumerate its models. That is not a failure of the
request, so it is a distinct variant rather than an error to log:

```rust
use everruns::{Provider, models};

# async fn run(provider: Provider, curated: Vec<String>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
let ids = match models::list(provider).await {
    Ok(catalog) => catalog.iter().map(|model| model.id().to_string()).collect(),
    // Keep the application's curated suggestions.
    Err(models::CatalogError::NoCatalog) => curated,
    Err(error) => return Err(error.into()),
};
# Ok(ids)
# }
```

`CatalogError::Call` carries the provider failure verbatim, with the full
`LlmError` decision intact.

### Metadata without a provider call

The profile registry is static data, so a model's identity can be read offline:

```rust
use everruns::{DriverId, models};

let profile = models::profile(&DriverId::OpenAI, "gpt-5.6-terra");
assert!(profile.is_some());
```

A `Model` that bundles its provider answers the same question directly with
`model.profile()`. A bare model id has no profile: nothing says which vendor's
registry to consult.

### Drivers and the vendor behind a provider

A provider's runtime key is the application's own name for it, so
`Provider::new("my-gateway", ...)` says nothing about which vendor's models it
serves. Declare the driver kind when they differ, and profile lookups resolve
against the vendor:

```rust
use everruns::{ChatDriver, DriverId, Provider};

# fn run(driver: impl ChatDriver + 'static) {
let provider = Provider::new("my-gateway", driver)
    .base_url("https://gateway.example/v1")
    .with_driver_id(DriverId::OpenAI);
# let _ = provider;
# }
```

Unset, the driver kind falls back to the runtime key, which is the conventional
case (`OpenAI::from_env()` and every driver crate's `from_env` already declare
it). A custom driver joins in by implementing `ChatDriver::list_models` and
returning `DiscoveredModel` values; returning `None` — the default — is how a
driver says it has no catalog. See [Custom providers](#custom-providers).

The runnable version of this section is
[`model_catalog.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/model_catalog.rs),
which runs offline without an API key.

## Custom providers

Use a custom provider when an application talks to a model service that the
Framework does not configure for you. The extension boundary is the public
`ChatDriver` trait plus a `Provider` value. The agent selects that provider's
model with a plain credential-free string id.

At a high level:

```rust
use everruns::{Agent, BuildError, ChatDriver, Provider};

fn agent_for(driver: impl ChatDriver + 'static) -> Result<Agent, BuildError> {
    Agent::builder()
        .instructions("Use the company model gateway.")
        .provider(Provider::new("company-gateway", driver))
        .model("assistant-v2")
        .build()
}
```

A driver implements the streaming chat-completion contract. It receives the
resolved endpoint, model-facing messages (`everruns::llm::Message`), and call
configuration, and returns an `LlmResponseStream`. Exact trait methods and event shapes live in the
[`everruns::ChatDriver` API reference](https://docs.rs/everruns/latest/everruns/trait.ChatDriver.html).

Keep credential lookup and refresh in trusted host/provider configuration.
Model ids must remain safe to log, compare, store, and pass across application
boundaries. Provider errors should preserve useful decisions without
including secrets.

A driver registered through a `DriverDescriptor` also declares which
environment variables it reads, on its own credential fields. Declaring is
inert — the driver never reads them — and it is what lets a caller resolve the
provider from the environment without any central name mapping. See
[Credentials](#credentials).

Use focused provider crates when they already implement the protocol you need.
Custom backends and provider registry topology belong to [low-level host
composition](/framework/custom-backends/), not ordinary model selection.
