---
title: Model Catalogs
description: Ask a provider which models it offers, and read what each one supports.
---

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

## What each entry carries

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

## From a selection to a run

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

## Providers without a catalog

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

## Metadata without a provider call

The profile registry is static data, so a model's identity can be read offline:

```rust
use everruns::{DriverId, models};

let profile = models::profile(&DriverId::OpenAI, "gpt-5.6-terra");
assert!(profile.is_some());
```

A `Model` that bundles its provider answers the same question directly with
`model.profile()`. A bare model id has no profile: nothing says which vendor's
registry to consult.

## Drivers and the vendor behind a provider

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
driver says it has no catalog. See
[Custom providers](/framework/custom-providers/).

The runnable version of this page is
[`model_catalog.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/model_catalog.rs),
which runs offline without an API key.
