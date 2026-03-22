# Embedding Specification

## Abstract

Everruns is embeddable through a shared `PlatformDefinition` composition root. An embedder can assemble a custom runtime surface, then pass the same definition to the control plane and worker so capabilities, LLM drivers, connection providers, and built-in harness templates stay aligned.

This spec defines the contract for embedding. See `crates/core/src/platform_definition.rs` for the public Rust API.

## Goals

1. Let downstream Rust services start from `ServerAppBuilder` and `WorkerAppBuilder` instead of forking Everruns binaries.
2. Let embedders add or remove built-in runtime components without patching internal call sites.
3. Keep server startup, worker execution, org initialization, and seeding consistent by reading from the same definition.
4. Preserve the existing OSS experience through default presets.

## PlatformDefinition

`PlatformDefinition` is the shared runtime bundle consumed by both server and worker. It currently owns:

- Capability registry
- LLM driver registry
- Connection-provider registry
- Built-in harness templates

The type lives in `everruns-core` so any binary can construct or mutate it without depending on `everruns-server`.

### Built-in Harness Templates

Built-in harness templates are data, not code plugins. Each template carries:

- Stable internal key
- Optional default-org seed UUID
- Name, description, system prompt, tags
- Built-in capability definitions with optional per-capability config
- Explicit roles

Roles replace hard-coded harness names for platform behavior:

- `Base`: fallback harness for session creation
- `Default`: org default harness for new orgs
- `Chat`: global chat harness

## Presets

Everruns provides default presets so OSS behavior remains unchanged unless an embedder overrides it.

### Server preset

`crates/server/src/platform.rs` defines:

- `oss_platform_definition()`
- `oss_platform_definition_for_grade()`
- `oss_connection_provider_registry()`
- `oss_built_in_harnesses()`

This preset centralizes the current OSS runtime surface, including inventory-discovered connection providers and built-in harness templates.

### Worker preset

`crates/worker/src/platform.rs` defines:

- `default_platform_definition()`
- `default_platform_definition_for_grade()`

The worker preset includes built-in capabilities and built-in LLM drivers. It intentionally leaves connection providers and harness templates empty because those are server-owned unless an embedder passes a fully shared definition.

## Consumption Rules

### Server

`ServerAppBuilder` accepts an optional `PlatformDefinition`. When absent, it uses the OSS preset.

The server must derive the following from the platform definition:

- Capability service registry
- Session service capability registry
- LLM driver registry for provider resolution and model sync
- Connection providers for user connection APIs
- Built-in harness templates for org initialization
- Seed behavior for providers, models, harnesses, and agents
- gRPC worker service capability/session registries
- DEV_MODE in-process worker registries

### Worker

`WorkerAppBuilder` accepts an optional `PlatformDefinition`. When absent, it uses the worker preset.

The worker must derive the following from the platform definition:

- Capability registry used during `reason` and `act`
- LLM driver registry used during `reason`
- gRPC worker adapter registries when adapter-based execution paths are used

## Seeding Contract

Seeding must honor the supplied platform definition.

### Required behavior

1. Built-in harness reconciliation must use the platform's built-in harness templates.
2. Provider seeding must skip providers whose driver type is not registered by the platform.
3. Model seeding must skip models whose provider driver is not registered by the platform.
4. Agent seeding must skip agents whose required capabilities are not registered by the platform.

This prevents broken seeded data such as agents referencing Daytona when Daytona was intentionally removed from the platform.

## Org Initialization Contract

Organization initialization must provision built-in harnesses from the supplied harness templates and resolve default/base settings through harness roles, not hard-coded harness names or global constants.

Default-org fixed UUIDs remain allowed for backward compatibility when a template includes `seed_id`.

## Builder Contract

### ServerAppBuilder

Embedders may extend the control plane by:

- Replacing the `PlatformDefinition`
- Adding routes
- Adding auth backends
- Adding event listeners
- Adding migrations
- Adding background tasks

### WorkerAppBuilder

Embedders may extend execution by:

- Replacing the `PlatformDefinition`
- Reusing Everruns worker runtime and lifecycle handling

## CLI Auth Extension Point

CLI authentication routes (`/v1/auth/cli/*`) are decoupled from any specific auth backend via `CliAuthState`. Embedders with external auth providers can mount CLI login without duplicating handler code.

Construct `CliAuthState` with your storage backend, `AuthState`, and URLs, then mount via `cli_auth_routes()`. `base_url` must include any API path prefix (e.g. `/api`) — no env-var lookup is performed at runtime:

```rust
use everruns_server::auth::cli_auth::{CliAuthState, cli_auth_routes};

let cli_state = CliAuthState {
    db: db.clone(),
    auth: auth_state.clone(),
    // Frontend origin for login page redirects
    frontend_url: "https://my-saas.example.com".into(),
    // Full backend URL including API prefix — used directly in callback URLs
    base_url: "https://my-saas.example.com/api".into(),
};
// In AuthBackend::auth_routes():
Some(my_auth_routes.merge(cli_auth_routes(cli_state)))
```

See `crates/server/src/auth/cli_auth.rs` for the full implementation.

## Non-goals

This spec does not require:

- Selective removal of built-in HTTP route modules
- Route-level OpenAPI composition for embedder routes
- Code-plugin harness implementations

Those may be added later, but they are outside the current embedding contract.

## Source Index

- `crates/core/src/platform_definition.rs`
- `crates/core/src/connection_provider.rs`
- `crates/server/src/auth/cli_auth.rs`
- `crates/server/src/app_builder.rs`
- `crates/server/src/platform.rs`
- `crates/server/src/seed.rs`
- `crates/server/src/org_init.rs`
- `crates/worker/src/app_builder.rs`
- `crates/worker/src/platform.rs`
- `crates/worker/src/durable_worker.rs`
- `crates/worker/src/activities.rs`
