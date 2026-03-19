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
