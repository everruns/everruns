---
type: Specification
title: "Embedding Specification"
description: "Embedding contract and `HostComposition`."
tags:
  - everruns
  - foundations
---
# Embedding Specification

## Abstract

Everruns is embeddable through a shared `HostComposition`, owned by `everruns-host` — the layer that executes a turn — rather than by the kernel (EVE-887). An embedder can assemble a custom runtime surface, then pass the same composition to the control plane and worker so capabilities, LLM drivers, and host services stay aligned. Hosted product services — built-in harness templates (EVE-881), the connector registry, and the system email sender (EVE-879) — are composed on `ServerAppBuilder` instead.

This spec defines the contract for embedding. See `crates/host/src/composition.rs` for the public Rust API.

## Goals

1. Let downstream Rust services start from `ServerAppBuilder` and `WorkerAppBuilder` instead of forking Everruns binaries.
2. Let downstream Rust services run application-facing agents through the
   Framework, while specialized hosts retain low-level in-process composition.
3. Let embedders add or remove built-in runtime components without patching internal call sites.
4. Keep server startup, worker execution, org initialization, and seeding consistent by reading from the same definition.
5. Preserve the existing OSS experience through default presets.

## HostComposition

`HostComposition` is the shared runtime bundle consumed by both server and worker. It currently owns:

- Capability registry
- LLM driver registry
- System egress service
- System utility LLM service
- Session filesystem factory
- Vector store

The type lives in `everruns-host`, the layer that consumes it, so constructing
an execution host does not make the kernel own deployment composition and does
not require depending on `everruns-server`.

Hosted control-plane services live on `ServerAppBuilder`, not on
`HostComposition`: `built_in_harnesses` (EVE-881), `connector_registry`,
and `email_sender` (EVE-879). The connector trait/registry and the email
contract live in `everruns-platform`.

Platform-wide host services are represented as provider-neutral traits or
factories. For example, `SessionFileSystemFactory` lets a platform select
in-memory, storage-backed, real-disk, or future S3-backed session files while
allowing each host to provide the concrete dependencies needed to resolve the
live filesystem.

## In-process Runtime

Applications that want to run agents in their own process, without the durable
engine or control-plane server, should use the application-facing `everruns`
crate and the Everruns Framework. This document owns the lower-level
`HostComposition` composition contract, not the normal application path.

Advanced hosts may use `everruns-host`, the only low-level host boundary. It
owns `HostComposition` and uses the shared `everruns-engine` planner, keeping
low-level in-process hosts aligned with worker behavior.

See [knowledge/framework/](../framework/) for application-facing ownership and
[the runtime specification](runtime.md) for the low-level host contract.

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

The rule is scoped to *built-in* harness identity. UUID literals used in tests, examples, or fixtures for org-owned (non-built-in) harnesses are unaffected. See [`knowledge/harnesses/harness-types.md`](../harnesses/harness-types.md) for the full rule and the narrow default-org seeding exception.

## Presets

Everruns provides default presets so OSS behavior remains unchanged unless an embedder overrides it.

### Server preset

`crates/server/src/platform.rs` defines:

- `oss_host_composition()`
- `oss_host_composition_for_grade()`
- `oss_connector_registry()`
- `oss_built_in_harnesses()`
- `system_email_sender()`

This preset centralizes the current OSS runtime surface, including inventory-discovered connectors and built-in harness templates (both composed on `ServerAppBuilder`, EVE-879/EVE-881).

### Worker preset

`crates/worker/src/platform.rs` defines:

- `default_host_composition()`
- `default_host_composition_for_grade()`

The worker preset includes built-in capabilities and built-in LLM drivers. Connectors, harness templates, and email are server-owned composition and never part of the worker surface.

## Consumption Rules

### Server

`ServerAppBuilder` accepts an optional `HostComposition`. When absent, it uses the OSS preset.

The server must derive the following from its host composition and explicit
server-owned product services:

- Capability service registry
- Session service capability registry
- LLM driver registry for provider resolution and model sync
- Connection providers for user connection APIs
- Built-in harness templates for org initialization
- Session filesystem factory for session file operations
- Seed behavior for providers, models, harnesses, and agents
- gRPC worker service capability/session registries
- DEV_MODE in-process worker registries

### Worker

`WorkerAppBuilder` accepts an optional `HostComposition`. When absent, it uses the worker preset.

The worker must derive the following from its host composition:

- Capability registry used during `reason` and `act`
- LLM driver registry used during `reason`
- gRPC worker adapter registries when adapter-based execution paths are used

## Seeding Contract

Seeding must honor the supplied host composition and server-owned templates.

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

- Replacing the `HostComposition`
- Adding routes
- Adding auth backends
- Adding event listeners
- Installing a vendor-neutral error reporter (see "Error Reporting Contract" below)
- Adding migrations
- Adding background tasks

### WorkerAppBuilder

Embedders may extend execution by:

- Replacing the `HostComposition`
- Installing a vendor-neutral error reporter (see "Error Reporting Contract" below)
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

## Error Reporting Contract

Embedders (SaaS wrappers) often want to plug a vendor error-reporting backend — Sentry, Datadog, Rollbar — into OSS. The embedding surface must stay vendor-neutral: OSS depends on no vendor SDK and has no vendor-specific types, env vars, or config names.

### Vendor-neutral trait

`everruns-core` defines:

- `ErrorReporter` — async trait with `report(ErrorReport)` (`crates/core/src/error_reporter.rs`)
- `ErrorReport` — severity + kind + message + `ErrorScope`
- `ErrorScope` — optional `user_id`, `org_id`, `session_id`, `request_id`, `route`, `component`, `task_id`, `workflow_id`, and freeform `extra` metadata
- `ErrorSeverity` — `Warning`, `Error`, `Fatal`
- `NoopErrorReporter` — default when no embedder reporter is installed

Implementations must be best-effort: a slow or failing reporter must never propagate into the request or task path. Spawn background work for heavy reporting if needed.

### Server hook

`ServerAppBuilder::error_reporter(reporter)` installs an embedder-provided reporter. When not installed, OSS defaults to `NoopErrorReporter`.

- `ServerContext::error_reporter` is always a `SharedErrorReporter` (never `None`) so custom routes, middleware, and background tasks can report into the same sink without branching.
- OSS internals report on well-known failure points (for example, stale-task reclamation errors). Reports from recurring background tasks are detached on a `tokio::spawn` so a stalled vendor endpoint cannot stall operational recovery.
- No OSS code references Sentry, Datadog, or any other vendor type.

### Worker hook

`WorkerAppBuilder::error_reporter(reporter)` installs the same reporter on the worker side. OSS forwards fatal `TaskWorker::run()` failures through the reporter with the `task_worker` component in scope. Fatal-path calls are bounded by a short timeout so a stalled reporter cannot block worker termination or supervisor restart.

Panic interception and per-task / per-workflow scoping are wrapper responsibilities today: wrappers that want that granularity wrap task execution themselves and emit their own reports. See **Not goals** below.

### UI hook

UI embedders install a wrapper-provided reporter via `ErrorReporterProvider` at the application root:

```tsx
import { ErrorReporterProvider, type ErrorReporter } from "everruns-ui/providers/error-reporter-provider";

const myReporter: ErrorReporter = {
  report(report) {
    // Map `report` onto the wrapper's vendor SDK (e.g. Sentry.captureException)
  },
};

<ErrorReporterProvider reporter={myReporter}>
  <App />
</ErrorReporterProvider>;
```

Feature code consumes the reporter via `useErrorReporter()` and never imports a vendor SDK. The default is a no-op reporter, so callers can always use the hook without null checks.

`ErrorScope` fields in TypeScript mirror the Rust contract one-to-one (including `taskId` and `workflowId`) so wrappers can share one mapping across server, worker, and UI. `ErrorReport.scope` is required on both sides (defaulting to an empty scope) for the same reason. `ErrorReporter.report` may return `void` or a `Promise<void>` so async wrappers (e.g., posting to an ingestion endpoint) work directly, and callers always treat it as fire-and-forget.

### Runtime context passed to wrappers

OSS provides the following to wrappers through the hook surface:

| Surface | Context fields provided |
| --- | --- |
| Server request | `request_id`, `route`, optional `user_id`, `org_id`, `session_id` (when bound), plus `extra` |
| Worker task | `task_id`, `workflow_id`, plus `extra` |
| UI | Whatever the wrapper attaches at the provider level plus the component path it emits from |

### Wrapper vs OSS responsibilities

- **OSS owns**: the `ErrorReporter` trait, `ErrorReport` / `ErrorScope` types, call-sites in internal failure paths, and the provider/hook surface.
- **Wrappers own**: vendor SDK initialization, secret management (DSNs, API keys), breadcrumb configuration, sampling, release tagging, environment metadata, and the concrete `ErrorReporter` implementation.

### Non-goals

- No Sentry SDK dependency or Sentry-specific types in OSS.
- No vendor-specific env vars (for example, `SENTRY_DSN`) in OSS.
- No SaaS-specific branching anywhere in OSS.
- No panic interception in OSS. Wrappers install their own panic hook if they want uncaught panics reported.
- No per-task / per-workflow automatic scoping from OSS worker internals today. Wrappers can attach those IDs themselves when they own task execution wrappers.

### Example: wiring a Sentry wrapper

```rust,ignore
use everruns_core::{ErrorReport, ErrorReporter};
use everruns_server::app_builder::ServerAppBuilder;
use async_trait::async_trait;
use std::sync::Arc;

struct SentryReporter;

#[async_trait]
impl ErrorReporter for SentryReporter {
    async fn report(&self, report: ErrorReport) {
        // Wrapper-owned: map ErrorReport onto Sentry SDK calls.
    }
}

ServerAppBuilder::new(config)
    .error_reporter(Arc::new(SentryReporter))
    .run()
    .await?;
```

`everruns-worker::WorkerAppBuilder::error_reporter` takes the same trait object and completes the worker-side of the contract.

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

## Org Creation Policy Extension Point

OSS owns organization creation (`POST /v1/orgs`). Wrappers that must run product policy before a tenant exists — verified-email gates, account/resource limits — supply that policy through OSS instead of forking the route. This keeps OSS as the owning boundary: wrappers provide policy, not a parallel create-org handler or a `/v1/saas/orgs` shadow endpoint.

`everruns-server` defines:

- `OrgCreatePolicy` — async trait with `check(OrgCreateContext) -> Result<(), OrgCreateRejection>`
- `OrgCreateContext` — borrows the authenticated `AuthUser` and the requested org name
- `OrgCreateRejection` — `status` + user-facing `message`, with an `OrgCreateRejection::forbidden(..)` helper for the common `403` case

`ServerAppBuilder::org_create_policy(Arc<dyn OrgCreatePolicy>)` installs the policy. When present, OSS runs `check` inside the create-org handler **before any org or membership row is written**. A rejection aborts creation with the rejection's status/body and persists nothing; the message is surfaced verbatim to the client, so it must be safe for UI display (e.g. `403 Please verify your email address before continuing.`). When no policy is registered, default OSS create-org behavior is unchanged.

```rust,ignore
use everruns_server::{OrgCreateContext, OrgCreatePolicy, OrgCreateRejection};
use everruns_server::app_builder::ServerAppBuilder;
use async_trait::async_trait;
use std::sync::Arc;

struct VerifiedEmailPolicy;

#[async_trait]
impl OrgCreatePolicy for VerifiedEmailPolicy {
    async fn check(&self, ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection> {
        // Wrapper-owned: consult the wrapper's identity provider for ctx.user.
        if wrapper_email_is_verified(ctx.user) {
            Ok(())
        } else {
            Err(OrgCreateRejection::forbidden(
                "Please verify your email address before continuing.",
            ))
        }
    }
}

ServerAppBuilder::new(config)
    .org_create_policy(Arc::new(VerifiedEmailPolicy))
    .run()
    .await?;
```

- **OSS owns**: the `OrgCreatePolicy` trait, `OrgCreateContext` / `OrgCreateRejection` types, the single pre-write call-site in the create-org handler, and the builder hook.
- **Wrappers own**: the concrete policy (verified-email checks, plan/resource limits), the data sources it consults, and the user-facing rejection copy.

This is the org-creation counterpart to the invitation surface moved into OSS by EVE-602; member management and invitations use the OSS surfaces directly.

## Org Initialization Extension Point

`OrgCreatePolicy` gates creation *before* an org exists; `OrgInitializer` provisions resources *after* one is created. Per-tenant provisioning is the normal shape of an embedded control plane — a managed provider, a default budget, a tenant row in an external system, a per-org key in a secret store. Without a hook, an embedder must reimplement every org-creation path or ship a background reconciler that repairs new orgs after the fact, leaving a window where the org is missing the resource.

`everruns-server` defines (in `org_init`):

- `OrgInitializer` — async trait with `on_org_created(OrgInitContext) -> anyhow::Result<()>`, plus `required(&self) -> bool` (default `true`) and `name(&self) -> &str` for diagnostics
- `OrgInitContext` — carries the storage backend (`db`), the new `org_id`, and `created_by: Option<Uuid>` (the requesting user, `None` for system-created orgs)
- `OrgInitializerError` — names the failed required initializer and wraps its source error

`ServerAppBuilder::org_initializer(Arc<dyn OrgInitializer>)` registers an initializer; it may be called multiple times and initializers run in registration order. OSS invokes them inside the create-org handler **after built-in harnesses and the default marketplace are provisioned**, so an initializer sees a fully set-up org. Failure policy is per-initializer:

- **Required** (default): a failure aborts creation — OSS rolls the org back best-effort (`delete_organization`) and returns `500`, so a caller never sees a "created" org missing host-mandated resources.
- **Optional** (`required() == false`): a failure is logged and creation proceeds.

When no initializer is registered, default OSS create-org behavior is unchanged.

```rust,ignore
use everruns_server::{OrgInitContext, OrgInitializer};
use everruns_server::app_builder::ServerAppBuilder;
use async_trait::async_trait;
use std::sync::Arc;

struct ProvisionManagedProvider;

#[async_trait]
impl OrgInitializer for ProvisionManagedProvider {
    async fn on_org_created(&self, ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
        // Wrapper-owned: create the host-managed provider row (its credential is
        // a per-org gateway token) for ctx.org_id via ctx.db.
        provision_gateway_provider(ctx.db, ctx.org_id).await
    }

    fn name(&self) -> &str {
        "managed-provider"
    }
}

ServerAppBuilder::new(config)
    .org_initializer(Arc::new(ProvisionManagedProvider))
    .run()
    .await?;
```

- **OSS owns**: the `OrgInitializer` trait, `OrgInitContext` / `OrgInitializerError` types, the single post-provision call-site in the create-org handler (with best-effort rollback), and the builder hook.
- **Wrappers own**: the concrete initializer, the resources it provisions, and its required/optional failure policy. Required initializers should stay idempotent so a retried creation converges.

This is the post-create counterpart to `OrgCreatePolicy`; both hook the same OSS create-org handler, keeping OSS the owning boundary for org lifecycle.

## Zero-Org Onboarding Surface

OSS owns the first screen a user sees after login when they have no organizations. Wrappers compose it instead of forking a full onboarding page plus the main-layout redirect.

OSS provides:

- `ZeroOrgOnboarding` (`apps/ui/src/components/onboarding/zero-org-onboarding.tsx`) — owns the layout, org-name form, current-org selection after create, and the `/orgs/{orgId}/setup` redirect. Default behavior lets any authenticated user create their first org.
- `useZeroOrgRedirect` (`apps/ui/src/components/onboarding/use-zero-org-redirect.ts`) — redirects authenticated zero-org users to `/onboarding`; consumed by the OSS `(main)` layout and reusable by wrappers so the redirect is not duplicated.
- The `(onboarding)/onboarding` route, which renders `ZeroOrgOnboarding`.

Wrappers configure two extension points on `ZeroOrgOnboarding`:

- `usePolicy: () => ZeroOrgPolicyState` returning `{ status: "loading" }`, `{ status: "ready" }`, or `{ status: "blocked", title, body?, actions? }`. A blocked state renders a gate (e.g. "Verify your email") in place of the form before any org is created. Defaults to always-ready.
- `useCreateOrg` — overrides the create mutation (e.g. to call a wrapper alias) while keeping the OSS layout and redirect. Defaults to the OSS `useCreateOrganization`.

A SaaS wrapper replaces its local onboarding page with an OSS re-export plus a small policy adapter:

```tsx
// Wrapper-owned policy adapter; the page is just OSS + this hook.
import { ZeroOrgOnboarding } from "everruns-ui/components/onboarding/zero-org-onboarding";

export default function Onboarding() {
  return <ZeroOrgOnboarding usePolicy={useVerifiedEmailPolicy} />;
}
```

- **OSS owns**: the `ZeroOrgOnboarding` component and layout, the `ZeroOrgPolicyState` / `UseZeroOrgPolicy` types, the always-ready default policy, the create-and-redirect flow, and the zero-org redirect hook.
- **Wrappers own**: the concrete policy hook (verified-email gate, plan limits) and any create-org override.

This is the UI counterpart to the server-side org-creation policy above; the policy-blocked copy a wrapper shows here should mirror the `OrgCreatePolicy` rejection that remains the authoritative backstop.

## Non-goals

This spec does not require:

- Selective removal of built-in HTTP route modules
- Route-level OpenAPI composition for embedder routes
- Code-plugin harness implementations

Those may be added later, but they are outside the current embedding contract.

## Source Index

- `crates/host/src/composition.rs`
- `crates/platform/src/connector.rs`
- `crates/core/src/error_reporter.rs`
- `apps/ui/src/providers/error-reporter-provider.tsx`
- `crates/server/src/auth/cli_auth.rs`
- `crates/server/src/api/organizations.rs`
- `crates/server/src/app_builder.rs`
- `crates/server/src/platform.rs`
- `crates/server/src/seed.rs`
- `crates/server/src/org_init.rs`
- `crates/worker/src/app_builder.rs`
- `crates/worker/src/platform.rs`
- `crates/worker/src/unified_worker.rs`
- `crates/worker/src/activities.rs`
- `apps/ui/src/lib/ui-route-manifest.ts`
- `apps/ui/package.json`
