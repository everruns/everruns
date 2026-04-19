# Embedding Specification

## Abstract

Everruns is embeddable through a shared `PlatformDefinition` composition root. An embedder can assemble a custom runtime surface, then pass the same definition to the control plane and worker so capabilities, LLM drivers, connection providers, and built-in harness templates stay aligned.

This spec defines the contract for embedding. See `crates/core/src/platform_definition.rs` for the public Rust API.

## Goals

1. Let downstream Rust services start from `ServerAppBuilder` and `WorkerAppBuilder` instead of forking Everruns binaries.
2. Let downstream Rust services run Everruns harnesses directly in-process through `everruns-runtime`.
3. Let embedders add or remove built-in runtime components without patching internal call sites.
4. Keep server startup, worker execution, org initialization, and seeding consistent by reading from the same definition.
5. Preserve the existing OSS experience through default presets.

## PlatformDefinition

`PlatformDefinition` is the shared runtime bundle consumed by both server and worker. It currently owns:

- Capability registry
- LLM driver registry
- Connection-provider registry
- Built-in harness templates

The type lives in `everruns-core` so any binary can construct or mutate it without depending on `everruns-server`.

## In-process Runtime

Embedders that want to execute harnesses in their own process, without the
durable engine or control-plane server, should use `everruns-runtime`.

`everruns-runtime` consumes the same `PlatformDefinition` type and uses the
shared core atoms and turn state machine. This keeps embedded execution aligned
with the worker/runtime behavior while exposing a simpler embedder-facing API.

See [specs/runtime.md](runtime.md) for the public embedded runtime contract.

### Built-in Harness Templates

Built-in harness templates are data, not code plugins. Each template carries:

- Stable `name` (the durable identifier — see "Harness Identity" below)
- Display name, description, system prompt, tags
- Built-in capability definitions with optional per-capability config
- Explicit roles

Roles replace hard-coded harness names for platform behavior:

- `Base`: fallback harness for session creation
- `Default`: org default harness for new orgs
- `Chat`: global chat harness

### Harness Identity

Built-in harnesses are identified by `name` (e.g. `base`, `generic`, `platform-chat`), not by UUID. Each org gets its own freshly-generated `harness_id` row at provisioning time; consumers must resolve built-in harnesses by name + `is_built_in`, never by hardcoded UUID literals.

The rule is scoped to *built-in* harness identity. UUID literals used in tests, examples, or fixtures for org-owned (non-built-in) harnesses are unaffected. See [`specs/harness-types.md`](harness-types.md) for the full rule and the narrow default-org seeding exception.

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

Each org receives freshly-generated `harness_id` rows. The default org is the only place where pinned UUIDs are tolerated, and that exception exists solely to keep historical seed rows stable; all consumer code must still resolve harnesses by `name` + `is_built_in`.

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

## UI Route Manifest Contract

UI embedders and SaaS wrappers must discover the OSS App Router tree through the published route manifest instead of mirroring `apps/ui/src/app` by hand.

### Stable export

`apps/ui/package.json` must export the manifest module at `everruns-ui/route-manifest`.

This module is a build-time/server-side contract. It is not part of the browser bundle.

### Manifest API

The manifest module lives at `apps/ui/src/lib/ui-route-manifest.ts` and exposes:

- `UI_ROUTE_MANIFEST_VERSION` - current schema version (`1`)
- `createUiRouteManifest()` - derives the current manifest from `src/app`
- `getOssUiRouteManifest()` - lazily returns the manifest for the bundled OSS route tree
- `mergeUiRouteManifests(base, overrides)` - applies wrapper override precedence
- `resolveUiRouteArtifactFilePath()` / `loadUiRouteArtifactModule()` - OSS helper APIs for build-time access to discovered artifacts

### Artifact shape

Each manifest artifact includes:

- `artifactId` - stable override key, formed from `appPath + ":" + fileName`
- `sourcePackage` - owning package name (`everruns-ui` for OSS artifacts)
- `kind` - App Router artifact kind (`page`, `layout`, `loading`, `error`, `template`, `route`, `not-found`, metadata assets)
- `appPath` - App Router tree path including route groups (example: `/(main)/dashboard`)
- `pathname` - public URL path with route groups removed (example: `/dashboard`)
- `fileName` - concrete artifact filename (example: `page.tsx`, `icon.svg`)
- `sourcePath` / `packageRelativePath` - stable package-relative source location under `src/app`

At the manifest level, `sourcePackage` identifies the package providing the effective override layer for the returned manifest. Artifact-level `sourcePackage` identifies the owner of each winning artifact.

`appPath` is required because public pathname alone is not enough to identify layout ownership. Route groups can share the same URL space while applying different `layout.tsx` or `error.tsx` files.

### Override precedence

Wrapper manifests override OSS manifests by exact `artifactId`.

- Same `artifactId`: wrapper artifact wins
- New `artifactId`: wrapper artifact is appended
- Missing wrapper artifact: OSS artifact remains in effect

This keeps OSS as the owner of the default route topology while allowing wrappers to replace specific routes or layouts intentionally.

### Wrapper-added routes

Wrappers add new routes by publishing additional manifest artifacts with new `artifactId` values, then merging them with the OSS manifest. They must not fork or manually duplicate the full OSS `src/app` tree just to preserve parity with OSS routes.

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
- `apps/ui/src/lib/ui-route-manifest.ts`
- `apps/ui/package.json`
