# Session Resource Registry

A generic, session-scoped registry of resources that are active alongside the
main conversation. Sandboxes, subagents, browser sessions, and any future
background work register here so the agent can query what is running and
infrastructure can discover what needs cleanup.

## Relationship to Leased Resources

**Leased resources** add lease/cleanup semantics (expiry, cleanup retry,
provider-specific teardown) on top of the registry. When a leased resource is
upserted, the `LeasedResourceStore` implementation auto-registers an entry in
the session resource registry. The registry is visibility; leased resources are
lifecycle management.

## Design

### Core trait — `SessionResourceRegistry`

Lives in `crates/core/src/traits.rs`. Available on `ToolContext` as
`session_resource_registry: Option<Arc<dyn SessionResourceRegistry>>`.

```
register(entry)          — any capability registers a resource
update_status(id, status) — mark completed/failed/released
get(session_id, id)      — look up one resource
list(session_id, filter?) — "what's running?"
deregister(session_id, id) — remove from registry
```

### Model — `SessionResourceEntry`

Lives in `crates/core/src/session_resource.rs`.

| Field          | Type                    | Description                                             |
|----------------|-------------------------|---------------------------------------------------------|
| resource_id    | String                  | Caller-provided stable ID (leased resource ID, session ID, etc.) |
| session_id     | SessionId               | Parent session                                          |
| kind           | String                  | Extensible: `"sandbox"`, `"subagent"`, `"browser_session"`, … |
| display_name   | String                  | Human-readable label                                    |
| status         | SessionResourceStatus   | Active / Completed / Failed / Released                  |
| metadata       | JSON                    | Kind-specific non-secret data                           |
| created_at     | DateTime                | When registered                                         |
| updated_at     | DateTime                | Last status change                                      |

`SessionResourceStatus`: `Active`, `Completed`, `Failed`, `Released`.

`resource_id` is unique per session — repeated calls with the same ID update
rather than duplicate.

### Registration contract

| Capability       | Registers when                        | kind              | resource_id source         |
|-----------------|---------------------------------------|--------------------|----------------------------|
| Sandbox (Daytona, E2B, Deno) | `LeasedResourceStore.upsert_resource` | `sandbox`         | Leased resource public ID  |
| Browser (Browserless)        | `LeasedResourceStore.upsert_resource` | `browser_session` | Leased resource public ID  |
| Subagents                    | `spawn_subagent` tool                 | `subagent`        | Child session public ID    |
| Sprites                      | `LeasedResourceStore.upsert_resource` | `sprite`          | Leased resource public ID  |
| *(future)*                   | Direct `registry.register()`          | *(any string)*    | Caller-defined             |

### Storage

Table `session_resources`:
```sql
CREATE TABLE session_resources (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    resource_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, resource_id)
);
```

Index: `idx_session_resources_session_status` on `(session_id, status)`.

`ON DELETE CASCADE` — when the session is removed, registry entries go with it.
For leased resources, the `leased_resources` table (which has `ON DELETE SET NULL`)
continues cleanup independently.

In-memory: `HashMap<SessionId, HashMap<String, SessionResourceEntry>>`.

### Agent visibility

Agents query via the registry through `ToolContext.session_resource_registry`.
Capabilities or tools can call `registry.list()` to answer "what is running?".
Infrastructure cleanup workers scan the registry to find stale resources.

### API

`GET /v1/sessions/{session_id}/resources` — returns `Vec<SessionResourceEntry>`.

### Runtime modes

| Mode   | Backend                                          |
|--------|--------------------------------------------------|
| Full   | PostgreSQL `session_resources` table              |
| Dev    | In-memory HashMap                                 |
| gRPC   | Follow-up: add `RegisterSessionResource` RPC     |

### Auto-registration from LeasedResourceStore

`DbLeasedResourceStore` takes an optional `Arc<dyn SessionResourceRegistry>`.
On `upsert_resource`, it also calls `registry.register()`. On `release_resource`,
it calls `registry.update_status(Released)`. This ensures all leased resources
appear in the registry without changing tool code.

### Feature flag

`LEASED_RESOURCES_FEATURE` gates UI tab visibility. Renamed concept: the tab
shows session resources (not just leased resources). Feature string stays
`"leased_resources"` for backward compat.
