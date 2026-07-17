---
title: Embedding Everruns
description: Build a custom Rust service on top of Everruns by composing your own PlatformDefinition, server, and worker
sidebar:
  order: 20
---

Everruns can be embedded into another Rust service instead of only running as the stock OSS binaries. The composition root is `PlatformDefinition`: a shared bundle of runtime components that both the control plane and worker can honor.

This lets you:

- Add your own API routes with `ServerAppBuilder`
- Add your own capabilities
- Remove built-in capabilities or connection providers you do not want
- Replace built-in harness templates
- Run the stock worker with the same runtime surface as your server
- Run Everruns fully in-process with `everruns-runtime` when you do not want the durable engine

## What PlatformDefinition Controls

`PlatformDefinition` currently owns:

- Capability registry
- LLM driver registry
- Connection-provider registry
- Built-in harness templates

The type lives in `everruns-core`, so you can build it without depending on server internals.

## Start From The OSS Preset

If you want "Everruns, but customized", start from the OSS preset and mutate it:

```rust
use std::sync::Arc;

use axum::{Router, routing::get};
use everruns_core::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole,
};
use everruns_server::{ServerAppBuilder, ServerConfig, oss_platform_definition};
use everruns_worker::{TaskWorkerConfig, WorkerAppBuilder};

fn custom_routes() -> Router {
    Router::new().route("/v1/ping", get(|| async { "pong" }))
}

fn platform() -> everruns_core::PlatformDefinition {
    let mut platform = oss_platform_definition();

    // Remove built-in components you do not want.
    platform.capability_registry_mut().unregister("daytona");
    platform.connection_providers_mut().unregister("daytona");

    // Replace the stock default/base harnesses with a smaller embedded preset.
    platform.built_in_harnesses_mut().retain(|harness| {
        !harness.has_role(BuiltInHarnessRole::Base)
            && !harness.has_role(BuiltInHarnessRole::Default)
    });
    platform.built_in_harnesses_mut().push(
        BuiltInHarnessDefinition::new(
            "minimal",
            "Minimal",
            "Small default harness for an embedded deployment.",
            "You are a helpful assistant.",
        )
        .with_roles([BuiltInHarnessRole::Base, BuiltInHarnessRole::Default])
        .with_tags(["minimal", "built-in"])
        .with_capabilities([BuiltInCapabilityDefinition::new("current_time")]),
    );

    platform
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let platform = platform();

    let server_platform = platform.clone();
    let worker_platform = platform.clone();

    tokio::spawn(async move {
        WorkerAppBuilder::new(TaskWorkerConfig::from_env())
            .platform_definition(worker_platform)
            .run()
            .await
            .expect("worker failed");
    });

    ServerAppBuilder::new(ServerConfig::from_env())
        .platform_definition(server_platform)
        .routes(custom_routes())
        .run()
        .await
}
```

## Build From Scratch

If you want a fully custom deployment, construct `PlatformDefinition` directly:

```rust
use everruns_core::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole,
    CapabilityRegistry, DriverRegistry, PlatformDefinition,
};

fn platform() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(everruns_core::CurrentTimeCapability);

    let mut drivers = DriverRegistry::new();
    everruns_openai::register_driver(&mut drivers);

    PlatformDefinition::builder()
        .capability_registry(capabilities)
        .driver_registry(drivers)
        .built_in_harnesses([
            BuiltInHarnessDefinition::new(
                "minimal",
                "Minimal",
                "Minimal embedded harness",
                "You are a helpful assistant.",
            )
            .with_roles([BuiltInHarnessRole::Base, BuiltInHarnessRole::Default])
            .with_capabilities([BuiltInCapabilityDefinition::new("current_time")]),
        ])
        .build()
}
```

When you build from scratch, only the components you register exist. Seeded providers, models, harnesses, and agents are filtered against that definition.

For in-process hosts, `InProcessRuntimeBuilder::new()` already starts with
`CapabilityRegistry::runtime_builtins()`. Supplying a custom
`PlatformDefinition` replaces that default. Use `runtime_builtins()` as your
base registry when you want the standard embedded-runtime capability surface and
then add or remove capabilities for your host.

## Custom LLM Provider Drivers

Built-in providers are registered with their crate's `register_driver`
(`everruns_openai::register_driver`, `everruns_anthropic::register_driver`, and
so on). To add a provider that is **not** compiled into `everruns-core` — for
example an OAuth-based gateway such as ChatGPT/Codex — register it under a custom
provider id with `register_external`. No core enum changes, and no hijacking a
built-in provider slot:

```rust
use everruns_core::driver_registry::{
    BoxedChatDriver, DriverConfig, DriverRegistry,
};

fn drivers() -> DriverRegistry {
    let mut drivers = DriverRegistry::new();
    everruns_openai::register_driver(&mut drivers); // built-ins

    // Embedder-defined provider, keyed by a custom id. The factory receives a
    // DriverConfig carrying everything it needs:
    //   config.api_key   — Option<String>
    //   config.base_url  — Option<String>
    //   config.metadata  — ProviderMetadata { refresh_token, account_id, extra }
    // External providers may authenticate via metadata instead of an api_key.
    drivers.register_external("openai-codex", |config: &DriverConfig| {
        Box::new(MyCodexDriver::new(config)) as BoxedChatDriver
    });

    drivers
}
```

Reference the provider from a model by its id, passing any non-API-key auth
through `provider_metadata`:

```rust
use everruns_core::{DriverId, ResolvedModel};
use everruns_core::driver_registry::ProviderMetadata;

let model = ResolvedModel {
    model: "gpt-5-codex".into(),
    provider_type: DriverId::External("openai-codex".into()),
    api_key: None, // external providers may use metadata-based auth instead
    base_url: None,
    provider_metadata: Some(ProviderMetadata {
        refresh_token: Some(oauth.refresh_token),
        account_id: Some(oauth.account_id),
        extra: None,
    }),
};
```

Notes:

- **Built-in vs custom ids.** Use the built-in ids (`openai`, `openrouter`,
  `azure_openai`, `openai_completions`, `anthropic`, `gemini`, `bedrock`, `mai`,
  `fireworks`, and `llmsim` for the test simulator) for the providers everruns
  ships. Reach for a
  custom `External` id only when adding a provider that is not in core — that
  keeps your driver from colliding with a built-in slot, and non-empty unknown
  ids round-trip cleanly through the database and the worker boundary instead of
  erroring. (An empty or whitespace provider id is rejected as a configuration
  error rather than becoming a nameless external provider.)
- **Replacing a built-in driver.** `register` panics on a duplicate provider id
  so double-registration surfaces loudly. To intentionally swap a built-in driver
  — for tests or a specialized deployment — use `register_or_replace`.
- **Case-insensitive ids.** External ids are normalized to lowercase, so
  registration and lookup agree regardless of the casing stored in the database
  or sent on the wire.
- **Fail-closed.** Drivers never silently fall back to process-environment
  credentials during a tenant turn; pass credentials explicitly via the model's
  `api_key` / `provider_metadata`.

## Built-in Harness Roles

Special harness behavior is driven by explicit roles, not fixed names:

- `Base`: used when session creation omits a harness and the org has no explicit base harness configured yet
- `Default`: set as the org default harness during initialization
- `Chat`: used by the global chat session endpoint

That means you can rename the harnesses freely. A platform can ship a base harness called `Minimal` or `Internal Default` as long as it carries the correct role.

## In-Process Runtime

If you do not want PostgreSQL, the control-plane server, or the worker boundary, use `everruns-runtime` — the agentic runtime, the in-process entrypoint to the Everruns agentic framework — instead of embedding the full server/worker stack. See the [Runtime](/features/runtime/) page for a quickstart; this section covers how it fits the embedding story.

This path is for applications that want to:

- run a harness entirely inside their own process
- register their own capabilities and drivers
- provide their own backend implementations
- inspect the assembled turn context before execution

```rust
use everruns_core::{
    CapabilityRegistry, DriverId, DriverRegistry, PlatformDefinition, ResolvedModel,
};
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_runtime::InProcessRuntimeBuilder;

let platform = PlatformDefinition::new(CapabilityRegistry::new(), DriverRegistry::new());

let runtime = InProcessRuntimeBuilder::new()
    .platform_definition(platform)
    .llm_sim(LlmSimConfig::fixed("hello from everruns-runtime"))
    .default_model(ResolvedModel {
        model: "llmsim-model".into(),
        provider_type: DriverId::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
        provider_metadata: None,
    })
    // .capability(...)
    // .backends(...)
    // .harness(...)
    // .session(...)
    .build()
    .await?;
```

### Context Inspection

`everruns-runtime` exposes `load_context(session_id)` so embedders can inspect the exact merged turn context that the reason phase will use. This works both before the first turn and after messages already exist:

- harness chain
- optional agent
- session
- filtered message history
- resolved model
- assembled `RuntimeAgent`

```rust
let context = runtime.load_context(session_id).await?;
println!("messages = {}", context.messages.len());
println!("model = {}", context.runtime_agent.model);
println!("tools = {}", context.runtime_agent.tools.len());
```

When the session has no messages yet, `context.messages` is empty and model/locale resolution falls back to merged harness, agent, session, and platform defaults.

Under the hood, execute-time hosts use `everruns_core::assemble_turn_context(...)`, while inspection uses `everruns_core::inspect_turn_context(...)`. Both share the same merged harness/agent/session assembly logic so in-process and worker-backed behavior stay aligned.

### Custom Backends

`everruns-runtime` ships in-memory defaults, but public extension traits let embedders supply their own stores:

- `RuntimeHarnessStore`
- `RuntimeAgentStore`
- `RuntimeSessionStore`
- `RuntimeMessageStore`
- `RuntimeFileStore`
- `RuntimeProviderStore`
- `EventBus` (extends `EventEmitter`; the default in-memory bus retains
  emitted events for `InProcessRuntime::events()`, while production buses
  inherit the default `collected_events` and return an empty `Vec`)

Pass them through `RuntimeBackends` on the builder:

```rust
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let runtime = InProcessRuntimeBuilder::new()
    .backends(
        RuntimeBackends::in_memory()
            .with_file_store(my_file_store)
            .with_message_store(my_message_store)
            .with_event_bus(my_event_bus),
        // Add other `.with_*` overrides as needed; defaults stay in-memory.
    )
    .build()
    .await?;
```

See also:

- `cargo run -p everruns-runtime --example in_process_runtime`
- `cargo run -p everruns-runtime --example inspect_context`
- `cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml` for a root-level host app example that defines its own capability, private tool data, seeded files, and in-process turn loop

## Seeding Behavior

Startup seeding now respects the supplied platform definition:

- Built-in harness reconciliation uses your harness templates
- Providers are only seeded when their driver exists
- Models are only seeded when their provider driver exists
- Agents are skipped when they require capabilities your platform removed

This is the mechanism that lets you remove Daytona from the platform without leaving broken seeded agents or connection-provider UI behind.

## Custom Routes

`ServerAppBuilder::routes()` still merges your routes into the stock API router. That is the simplest way to add platform-specific endpoints such as billing, internal admin APIs, or product-specific workflows.

```rust
use axum::{Json, Router, routing::post};
use serde_json::json;

fn routes() -> Router {
    Router::new().route(
        "/v1/internal/reindex",
        post(|| async { Json(json!({ "queued": true })) }),
    )
}
```

## Current Boundary

The current embedding contract is strong around runtime surface composition, but it does not yet cover everything:

- Stock HTTP modules are still mounted by default
- Custom routes do not contribute to OpenAPI automatically
- Harnesses are data templates, not runtime code plugins

For most embedders, that is enough to build a custom control plane on top of Everruns while still reusing the durable execution engine, workers, and platform services.
