---
type: Specification
title: "Model Router Specification"
description: "Model Routers."
tags:
  - everruns
  - integrations
---
# Model Router Specification

## Abstract

Model Routers provide semantic LLM selection. A router owns named **routes**
(e.g. `base`, `utility`, `analysis`, `review`); each route has a human-facing
`purpose`, a model-facing `when_to_use` description, a selection strategy, and
candidates with concrete model references plus request overrides like
reasoning effort. Harnesses, agents, sessions, and org settings can bind to
either a concrete model (today's behavior) or a router (new); routers can
also accept caller-supplied parameters validated against a router-defined
schema.

This spec is the durable design intent for [EVE-397]. Implementation is
delivered across multiple PRs; the entity, ID schema, capability registration
adjacent surface, and DB schema land in the foundational PR. Storage trait,
CRUD APIs, runtime resolver, binding migrations on harnesses/agents/sessions,
and UI follow as additional vertical slices.

## Motivation

Today, every place that selects an LLM stores a concrete model ID
(`agent.default_model_id`, `harness.default_model_id`, `session.model_id`).
That works for fixed pipelines but does not express *why* a model was chosen
or let users adjust selection without editing every binding:

- A team that wants "use the fast model for utility, the smart model for
  analysis, and the deliberate model for review" has to encode that across
  many entities and update them when models change.
- Operators have no way to express fallback ("use Sonnet but fall back to
  Opus on rate-limit") or weighted A/B selection.
- Per-environment differences (dev vs prod model choice) require code-level
  rebinding instead of configuration.
- Embedded runtimes have no extension point for custom routing logic.

Model Routers introduce a durable, named layer between *intent* (a route
like `analysis`) and *implementation* (a concrete model invocation with
request overrides), shared across all binding sites.

## Concepts

| Name                       | Description                                                                                                                                                          |
|----------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Model Router**           | Org-scoped named container of routes plus a parameter schema. Public ID prefix: `mrtr_`.                                                                              |
| **Route**                  | Named selection target inside a router (e.g. `base`, `analysis`). Has `key`, `purpose`, `when_to_use`, `strategy`, candidates.                                       |
| **Candidate**              | Ordered choice inside a route: a concrete `model_id` plus optional request overrides (`reasoning_effort`, `temperature`, `max_output_tokens`, etc.) and weight/rules. |
| **Strategy**               | How to pick a candidate inside a route: `single`, `ordered_fallback`, `weighted`, `rules`, or `custom`.                                                              |
| **Binding**                | Reference from an org/harness/agent/session to either a concrete `model_id` (existing) or a router. Routers may also carry caller-supplied `params`.                  |
| **Router Parameters**      | JSON Schema-typed values passed by the binding (e.g. `{"tier": "fast"}`); used by `rules` strategies and by `custom` resolvers.                                       |

`purpose` is the human-facing label visible in UI ("what is this route for").
`when_to_use` is the model-facing description used in the future `set_model`
tool's discoverability text and in route-preview UI. They are flat fields on
the route, not wrapped in a `semantic` object.

## Lifecycle

Routers follow the standard building-block lifecycle from `knowledge/foundations/models.md`:

* `active`, assignable, editable, listed by default.
* `archived`, read-only, hidden from default lists, not assignable to new
  bindings; existing bindings continue resolving.
* `deleted`, tombstone; detail/list APIs return 404 except for historical
  references.

Routes and candidates live as child rows under a router. Editing a route or
candidate bumps the router's `updated_at`; deleting a router cascades.

## Data Model

### `model_routers`

| Column                    | Type        | Notes                                                  |
|---------------------------|-------------|--------------------------------------------------------|
| `id`                      | UUID PK     | Internal primary key.                                  |
| `org_id`                  | BIGINT FK   | Organization scope (matches existing `agents` pattern, no cascade). |
| `public_id`               | TEXT        | `mrtr_<32-hex>`. Unique per `org_id`.                  |
| `name`                    | VARCHAR     | Unique within `org_id` while not deleted.              |
| `description`             | TEXT?       |                                                        |
| `param_schema`            | JSONB       | JSON Schema describing caller-supplied params (default `{}`). |
| `status`                  | VARCHAR     | `active` / `archived` / `deleted`.                     |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                                        |
| `archived_at` / `deleted_at` | TIMESTAMPTZ? |                                                       |

### `model_router_routes`

| Column          | Type        | Notes                                       |
|-----------------|-------------|---------------------------------------------|
| `id`            | UUID PK     |                                             |
| `router_id`     | UUID FK     | `ON DELETE CASCADE`.                        |
| `key`           | VARCHAR     | Stable identifier within router (e.g. `base`, `analysis`). Unique per router. |
| `purpose`       | TEXT        | Human-facing summary.                       |
| `when_to_use`   | TEXT        | Model-facing description for future `set_model` discoverability. |
| `strategy`      | VARCHAR     | `single` / `ordered_fallback` / `weighted` / `rules` / `custom`. |
| `position`      | INTEGER     | Display order within router.                |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                 |

### `model_router_candidates`

| Column            | Type        | Notes                                       |
|-------------------|-------------|---------------------------------------------|
| `id`              | UUID PK     |                                             |
| `route_id`        | UUID FK     | `ON DELETE CASCADE`.                        |
| `model_id`        | UUID FK     | `REFERENCES llm_models(id)`. The concrete model to invoke. |
| `request_overrides` | JSONB     | Provider-agnostic overrides (`reasoning_effort`, `temperature`, `max_output_tokens`, ...). |
| `weight`          | INTEGER     | Used by `weighted` strategy; default `1`.   |
| `rules`           | JSONB?      | Used by `rules` strategy. Free-form rules document validated by the server. |
| `position`        | INTEGER     | Used by `ordered_fallback` strategy.        |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                  |

Bindings on `harnesses`, `agents`, `sessions`, and org settings extend the
existing concrete-`model_id` columns with a parallel `model_router_id` column
plus a `model_router_params` JSONB column. Exactly one of
`{default_model_id, model_router_id}` may be set at a time per binding
site. The migration that adds those columns ships in the next vertical slice
(see "Out of scope here") to keep this foundation PR small.

## Strategies

| Strategy           | Behavior                                                                                                                                  |
|--------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| `single`           | Exactly one candidate (no more, no less); trivial selection. Useful as a starting point and for direct routes.                            |
| `ordered_fallback` | Try candidates in `position` order. Fall through to the next candidate on transient errors (rate limit, timeout, retryable provider error). |
| `weighted`         | Sample a candidate by `weight` (per-call random selection). Useful for A/B and canary rollouts.                                           |
| `rules`            | Evaluate the candidate's `rules` against the binding's `params` (e.g. `{ "if": { "tier": "fast" }, "then": "model_xyz" }`). First match wins. |
| `custom`           | Hand off to an embedded resolver registered by the host runtime via the resolver-extension point. Database stores the candidate list as advisory metadata. |

The runtime resolver and the strategy implementations themselves ship in a
follow-up vertical slice. This PR validates that strategy values are one of
the five enum members and parses them into typed values.

### OpenRouter Routing Bridge

The foundational router types also define a pure OpenRouter bridge for the
strategies that map directly to OpenRouter request-level routing extensions.
When every candidate resolves to an OpenRouter model slug, `single` compiles to
the primary `model` field with no provider-specific routing, and
`ordered_fallback` compiles to the first candidate as `model` plus OpenRouter
`models` and `route: "fallback"` fields in candidate `position` order.

`weighted`, `rules`, and `custom` remain Everruns resolver responsibilities
because OpenRouter's fallback router does not express local sampling,
parameterized rules, or host-registered custom logic. Future storage-backed
resolver slices can use this bridge after they have validated org scope,
provider type, and concrete model availability.

## Resolution Contract

At LLM-call time the resolver returns:

```rust
struct ResolvedModelInvocation {
    model_id: ModelId,
    request_overrides: serde_json::Value,
    // Provenance for observability and tracing
    via_router: Option<ModelRouterId>,
    via_route_key: Option<String>,
    via_candidate_id: Option<Uuid>,
}
```

Resolution order at any binding site:

1. If the binding has a concrete `default_model_id`, return it directly with
   no overrides, current behavior, preserved for backward compatibility.
2. If the binding has a `model_router_id`, look up the router's route. The
   route key is determined by the caller (initial implementation: a single
   `base` route). Apply the strategy to candidates with the binding's
   `params` and the router's `param_schema`-validated values.
3. If resolution fails (no candidates, custom resolver returns nothing),
   surface a structured `ModelRoutingError` rather than panicking.

A future vertical slice extends agent loops with a `set_model(route_key)`
tool so an agent can hop between named routes (e.g. switch from `base` to
`analysis`) at LLM-step boundaries.

## API

REST endpoints (full surface ships in the CRUD slice):

* `GET    /v1/model-routers`
* `POST   /v1/model-routers`
* `GET    /v1/model-routers/{router_id}`
* `PATCH  /v1/model-routers/{router_id}`
* `DELETE /v1/model-routers/{router_id}`, archive / delete per lifecycle
* `GET    /v1/model-routers/{router_id}/routes`
* `POST   /v1/model-routers/{router_id}/routes`
* `PATCH  /v1/model-routers/{router_id}/routes/{route_id}`
* `DELETE /v1/model-routers/{router_id}/routes/{route_id}`
* `POST   /v1/model-routers/{router_id}/routes/{route_id}/candidates`
* `PATCH  /v1/model-routers/{router_id}/routes/{route_id}/candidates/{candidate_id}`
* `DELETE /v1/model-routers/{router_id}/routes/{route_id}/candidates/{candidate_id}`
* `POST   /v1/model-routers/{router_id}/preview`, given `params`, return the resolved `model_id` and overrides per route (no LLM call).

Existing model-binding endpoints on harnesses/agents/sessions accept either a
concrete `model_id` or a `model_router_id` + optional `params`. Server-side
validation ensures exactly one is set.

## UI

* New **Model Routers** settings view near Models / Providers.
* Router list shows route chips, strategy badges, and a default/usage
  indicator.
* Router editor: metadata (name, description, `param_schema`), routes table
  with candidates and request overrides, resolve preview that takes
  per-route params.
* Binding UI on org/harness/agent/session: segmented control (Concrete model
  / Model router); when router is selected, render parameter form from the
  router's `param_schema` and preview the resolved route.

## Security & Permissions

See `knowledge/security/threat-model.md` for the canonical entries.

* **Cross-org binding:** Validation MUST reject `model_router_id` references
  from other orgs. Errors must not leak existence.
* **Privilege via overrides:** `request_overrides` cannot raise the trust
  level of a model invocation; they are passed through to the existing LLM
  driver request and are subject to the same audit and rate-limit paths.
* **Custom resolvers:** Embedded runtimes register resolvers in code, not
  through the API; database `custom` strategy metadata is advisory only.
* **Audit:** Router create/update/archive/delete and binding changes are
  audited via the standard audit log domain.

## Resolver Extension Point

Embedded runtimes (CLI, local SDKs, other hosts) may register a custom
resolver implementing a stable trait, e.g.

```rust
#[async_trait]
pub trait ModelRouterResolver: Send + Sync {
    async fn resolve(
        &self,
        ctx: &ModelRoutingContext,
    ) -> Result<ResolvedModelInvocation, ModelRoutingError>;
}
```

Hosts can replace database-backed routers entirely (returning resolutions
purely from in-process configuration) or layer their resolver above the
database one. The trait, the registry, and the default DB-backed resolver
live in `crates/core` and `crates/server` respectively and ship in the
runtime-resolver vertical slice.

## Out of Scope Here (Foundation PR)

The first PR delivering this spec lands:

* `knowledge/integrations/model-router.md` (this document).
* `ModelRouterId` typed id (`mrtr_<32-hex>`). Route and candidate IDs remain
  raw `Uuid` values in the foundation PR; typed IDs for them can land later
  if a binding-site needs to surface them externally.
* Migration `026_model_routers.sql` (tables `model_routers`,
  `model_router_routes`, `model_router_candidates`).
* `crates/core/src/model_router.rs`, entity types, strategy enum, candidate
  shape, structural validation (route key format, strategy parse, candidate
  must reference a model, weight non-negative).
* CHANGELOG entry.

Out of scope for the foundation PR (each gets a separate slice):

1. Storage trait + in-memory + Postgres impls.
2. Domain commands/queries.
3. REST API for router CRUD and route/candidate CRUD.
4. Binding migration: add `model_router_id` + `model_router_params` columns
   to `harnesses`, `agents`, `sessions`, and org settings.
5. Runtime resolver: trait, default DB-backed implementation,
   strategy implementations, integration with LLM driver invocation.
6. Resolver extension-point registry for embedded runtimes.
7. UI: Model Routers settings view, router editor, binding UI updates.
8. `set_model` agent tool.

## Open Questions

* Route-key vocabulary: should a default registry of route keys (`base`,
  `utility`, `analysis`, `review`) be reserved, or should orgs be free to
  invent keys? (Tracked on EVE-397.)
* Should `request_overrides` validate against a stable, provider-agnostic
  shape, or accept any JSON and let the LLM driver reject unknown fields?
* Is observability provenance (`via_router`, `via_route_key`,
  `via_candidate_id`) part of the public event payload from day one?

[EVE-397]: https://linear.app/everruns/issue/EVE-397
