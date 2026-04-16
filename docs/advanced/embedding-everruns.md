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
use everruns_worker::{WorkerAppBuilder, DurableWorkerConfig};

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
        WorkerAppBuilder::new(DurableWorkerConfig::from_env())
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

## Built-in Harness Roles

Special harness behavior is driven by explicit roles, not fixed names:

- `Base`: used when session creation omits a harness and the org has no explicit base harness configured yet
- `Default`: set as the org default harness during initialization
- `Chat`: used by the global chat session endpoint

That means you can rename the harnesses freely. A platform can ship a base harness called `Minimal` or `Internal Default` as long as it carries the correct role.

## In-Process Runtime

If you do not want PostgreSQL, the control-plane server, or the worker boundary, use the public `everruns-runtime` crate instead of embedding the full server/worker stack.

This path is for applications that want to:

- run a harness entirely inside their own process
- register their own capabilities and drivers
- provide their own backend implementations
- inspect the assembled turn context before execution

```rust
use everruns_core::{
    CapabilityRegistry, DriverRegistry, LlmProviderType, ModelWithProvider, PlatformDefinition,
};
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_runtime::InProcessRuntimeBuilder;

let platform = PlatformDefinition::new(CapabilityRegistry::new(), DriverRegistry::new());

let runtime = InProcessRuntimeBuilder::new()
    .platform_definition(platform)
    .llm_sim(LlmSimConfig::fixed("hello from everruns-runtime"))
    .default_model(ModelWithProvider {
        model: "llmsim-model".into(),
        provider_type: LlmProviderType::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
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

Pass them through `RuntimeBackends` on the builder:

```rust
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let runtime = InProcessRuntimeBuilder::new()
    .backends(RuntimeBackends {
        harness_store: my_harness_store,
        agent_store: my_agent_store,
        session_store: my_session_store,
        message_store: my_message_store,
        provider_store: my_provider_store,
        event_emitter: my_event_emitter,
        event_collector: None,
        file_store: my_file_store,
        storage_store: my_storage_store,
        memory_store: my_memory_store,
    })
    .build()
    .await?;
```

See also:

- `cargo run -p everruns-runtime --example in_process_runtime`
- `cargo run -p everruns-runtime --example inspect_context`

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
