---
type: Specification
title: "Session Resource Registry"
description: "Session resource registry."
tags:
  - everruns
  - runtime-resources
---
# Session Resource Registry

A generic, session-scoped registry of infrastructure resources that are active
alongside the main conversation — sandboxes, browser sessions, voice
connections — so the agent can query what is held and infrastructure can
discover what needs cleanup.

Background **work** (subagents, external agent runs, background tool runs) is
tracked by the session task registry instead; see
[`knowledge/runtime-resources/session-tasks.md`](session-tasks.md). Resources are *held and
released*; tasks *run and finish*.

## Relationship to Leased Resources

**Leased resources** add lease/cleanup semantics (expiry, cleanup retry,
provider-specific teardown) on top of the registry. When a leased resource is
upserted, the `LeasedResourceStore` implementation auto-registers an entry in
the session resource registry. The registry is visibility; leased resources are
lifecycle management.

## Design

### Core trait — `SessionResourceRegistry`

Lives in `crates/core/src/session_services.rs`. Available on `ToolContext` as
`session_resource_registry: Option<Arc<dyn SessionResourceRegistry>>`.

```
register(entry)                       — any capability registers a resource
update_status(session_id, id, status) — mark completed/failed/released
get(session_id, id)                   — look up one resource
list(session_id, filter?)             — "what's running?"
deregister(session_id, id)            — remove from registry
```

### Model — `SessionResourceEntry`

See `crates/core/src/session_resource.rs` for the full `SessionResourceEntry` struct and `SessionResourceStatus` enum.

`resource_id` is unique per session — repeated calls with the same ID update
rather than duplicate.

### Registration contract

| Capability       | Registers when                        | kind              | resource_id source         |
|-----------------|---------------------------------------|--------------------|----------------------------|
| Sandbox (Daytona, E2B, Deno) | `LeasedResourceStore.upsert_resource` | `sandbox`         | Leased resource public ID  |
| Browser (Browserless)        | `LeasedResourceStore.upsert_resource` | `browser_session` | Leased resource public ID  |
| Sprites                      | `LeasedResourceStore.upsert_resource` | `sprite`          | Leased resource public ID  |
| Voice Connections            | Voice bootstrap endpoints             | `voice_connection` | Voice connection public ID |
| *(future)*                   | Direct `registry.register()`          | *(any string)*    | Caller-defined             |

Work-shaped kinds (`subagent`, `agent_run`, `background_run`, `agent_handoff`)
were migrated to `session_tasks` (migration 053). The dual-write transitional
registrations were retired in migration 054; these kinds are no longer written
to `session_resources`.

### Storage

See `crates/server/migrations/` for the `session_resources` table DDL.

Key constraints: `UNIQUE(session_id, resource_id)`, `ON DELETE CASCADE` from sessions (registry entries removed with session; leased resources table continues cleanup independently).

In-memory: `HashMap<SessionId, HashMap<String, SessionResourceEntry>>`.

### Agent visibility

Agents query via the registry through `ToolContext.session_resource_registry`.
Capabilities or tools can call `registry.list()` to answer "what is running?".
Infrastructure cleanup workers scan the registry to find stale resources.

### Background runs

Background tool runs are session resources, not leased resources.

Properties:
- They are registered immediately when `spawn_background` accepts the run.
- They remain visible while active and are updated in-place as status, output tail, or progress changes.
- Metadata may include:
  - `tool`
  - `status_text`
  - `progress`
  - `output_tail`
  - `log_path`
  - `result_path`
  - `summary`
- Final logs and result payloads live in the session VFS under `/.background/{run_id}/`.

V1 limitation:
- The registry gives visibility only. It does not make background runs durable or restartable.
- If a worker dies, the registry entry may remain until cleanup logic or a later reconciliation marks it failed.

### API

`GET /v1/sessions/{session_id}/resources` — returns `Vec<SessionResourceEntry>`.

### Runtime modes

| Mode   | Backend                                          |
|--------|--------------------------------------------------|
| Full   | PostgreSQL `session_resources` table              |
| Dev    | In-memory HashMap                                 |
| gRPC   | `RegisterSessionResource`, `UpdateSessionResourceStatus`, `ListSessionResources`, `DeregisterSessionResource` RPCs |

### Auto-registration from LeasedResourceStore

`DbLeasedResourceStore` takes an optional `Arc<dyn SessionResourceRegistry>`.
On `upsert_resource`, it also calls `registry.register()`. On `release_resource`,
it calls `registry.update_status(Released)`. This ensures all leased resources
appear in the registry without changing tool code.

Reserved metadata keys added during auto-registration:
- `leased_resource_provider`
- `leased_resource_type`
- `leased_resource_external_id`
- `leased_resource_id`

These keys let tools verify that a provider-owned external ID belongs to the
active session even in runtimes that only expose the generic session resource
registry and not the leased-resource store directly.

### Feature flag

`LEASED_RESOURCES_FEATURE` gates UI tab visibility. Renamed concept: the tab
shows session resources (not just leased resources). Feature string stays
`"leased_resources"` for backward compat.
