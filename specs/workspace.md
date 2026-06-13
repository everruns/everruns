# Workspace Specification

## Abstract

This document defines the **Workspace** — an org-scoped, named, durable
working area that contains an agent's active state for a task. A Workspace
holds the files an agent reads and writes during execution, exposed at the
`/workspace` mount point. Capabilities (`file_system`, `virtual_bash`) read
and write through the `WorkspaceFileSystem` seam.

Workspaces are addressable entities (`wsp_<32-hex>`). Sessions attach to a
Workspace; multiple sessions may attach to the same Workspace (for shared
working state) or each session may own its own Workspace (the default,
matching legacy behavior).

## Model

Workspace is the **session tier** of the memory model:

* **Workspace** (this spec) — named, durable working area. One or more
  sessions attach to a Workspace. Files are workspace-scoped.
* **Memory** (`specs/memory.md`) — org-scoped, named, durable shared
  store. Mounted into a Workspace via the `memory` capability at session
  creation, RO by default.

Surfaces of a Workspace today:

* **Files** — virtual filesystem mounted at `/workspace`, this spec.
* **Tables** — session-scoped SQL databases (`specs/session-sqldb.md`)
  remain session-scoped for now; future work may promote them to a
  Workspace surface.

Future surfaces (key-value, secrets) are out of scope.

## Lifecycle

Workspaces follow the standard building-block lifecycle from
`specs/models.md`:

* `active` — assignable to sessions, editable, listed by default.
* `archived` — hidden from default lists, not assignable to new sessions,
  files become read-only.
* `deleted` — tombstone; detail/list APIs return 404 except for historical
  references (existing session links).

A Workspace is hard-deleted only after all attached sessions are
removed; otherwise `DELETE` archives the row.

## Default Workspace per Session

For backward compatibility, a session created without an explicit
`workspace_id` auto-creates a **default Workspace** named after the session
(`session-<session-id-suffix>`) whose primary key equals the session id (the
equality invariant), and attaches the session to it.

Power users who want to share working state across sessions pre-create a
Workspace via the API and pass its `workspace_id` (the `wsp_<32-hex>` public id)
to `CreateSession`. The attach target is validated at create time: it must
exist in the caller's org (else `404`) and be `active` (an `archived`/`deleted`
target is rejected `400`). When attached, no default workspace is created and
the session's `workspace_id` points at the shared workspace.

File access re-keys by the session's `workspace_id`, not its id, so a shared
workspace is addressed consistently everywhere: the agent's `file_system` /
`virtual_bash` tools (a [`WorkspaceScopedFileSystem`] decorator pins tool I/O to
the workspace), system-prompt file reads, initial-file seeding and capability
mounts, and the legacy `/v1/sessions/{session_id}/fs/*` alias. For the default
1:1 session these all reduce to the session id, so the re-keying is a
transparent pass-through.

This preserves the legacy 1:1 session→files invariant for clients that
don't yet think about Workspaces, while exposing the multi-session-share
model to API clients that opt into it.

## Data Model

### `workspaces`

| Column                    | Type        | Notes                                                  |
|---------------------------|-------------|--------------------------------------------------------|
| `id`                      | UUID PK     | Internal primary key.                                  |
| `org_id`                  | BIGINT FK   | Organization scope.                                    |
| `public_id`               | TEXT        | `wsp_<32-hex>`. Globally unique.                       |
| `name`                    | VARCHAR     | Unique within `org_id` while not deleted.              |
| `description`             | TEXT?       |                                                        |
| `owner_principal_id`      | TEXT?       | Principal that created the workspace.                  |
| `resolved_owner_user_id`  | UUID?       | Resolved user id, when known.                          |
| `status`                  | VARCHAR     | `active` / `archived` / `deleted`.                     |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                                       |
| `archived_at` / `deleted_at` | TIMESTAMPTZ? |                                                     |

`UNIQUE(public_id)` (globally), `UNIQUE(org_id, public_id)`, and
`UNIQUE(org_id, name) WHERE status != 'deleted'`. The global `public_id`
uniqueness backs `get_workspace_organization_id`, which resolves the owning
org from the opaque `wsp_<32-hex>` alone (used by routes that take a
workspace_id without an org context).

### `sessions`

Adds `workspace_id UUID NOT NULL REFERENCES workspaces(id)`.

Sessions cannot exist without a Workspace. Existing rows are backfilled by
the migration: one Workspace is created per pre-existing session, named
`session-<full-32-hex>` (the full UUID-without-dashes — UUIDv7 prefixes are
time-derived and collide under bursty creation, which would violate the
per-org name uniqueness constraint), and the session's `workspace_id` is set.

### `workspace_files`

Renamed from `session_files`. The FK column is renamed `session_id` →
`workspace_id` and now references `workspaces(id)`. All other shape
constraints (path validation, content/directory semantics, etc.) are
preserved verbatim.

## API

REST endpoints:

* `GET    /v1/workspaces`
* `POST   /v1/workspaces`
* `GET    /v1/workspaces/{workspace_id}`
* `PATCH  /v1/workspaces/{workspace_id}`
* `DELETE /v1/workspaces/{workspace_id}` — archive / delete per lifecycle
* `GET    /v1/workspaces/{workspace_id}/fs/...`
* `POST   /v1/workspaces/{workspace_id}/fs/...`
* `PUT    /v1/workspaces/{workspace_id}/fs/...`
* `DELETE /v1/workspaces/{workspace_id}/fs/...`
* `POST   /v1/workspaces/{workspace_id}/fs/_/{move,copy,stat,grep}`
* `GET    /v1/workspaces/{workspace_id}/fs/_/download/...` — raw bytes

A `GET` on a file path also returns raw bytes when the request carries
`Accept: application/octet-stream`; otherwise it returns the JSON envelope.
The whole `/fs/*` surface (the path routes plus the `/fs/_/*` action routes)
is mirrored on the legacy `/v1/sessions/{session_id}/fs/*` aliases for
backwards compatibility.

### Session-scoped aliases

For backward compatibility, the legacy session-scoped filesystem routes are
preserved as **thin aliases** that resolve the session's `workspace_id` and
delegate to the workspace routes:

* `/v1/sessions/{session_id}/fs/...` → `/v1/workspaces/{session.workspace_id}/fs/...`

Behavior, request body, and response shape are identical. Clients that hold
session IDs do not need to change.

## Design Decisions

### Decision 1: PostgreSQL-backed Storage
**Chosen:** Store files in PostgreSQL BYTEA column
**Alternatives considered:**
- Object storage (S3, MinIO): Added complexity for MVP
- Local filesystem: Not suitable for distributed deployments
**Rationale:** PostgreSQL provides ACID transactions, simple deployment, and good performance for small-to-medium files. Can migrate to object storage later for large files.

### Decision 2: RESTful Path-based API
**Chosen:** `/fs/{path}` with HTTP methods mapping to operations
**Rationale:** Path-based URLs are intuitive. GET reads, POST creates, PUT updates, DELETE removes. Action routes use the reserved `_/` prefix.

### Decision 3: Workspace Ownership Model
**Chosen:** Sessions attach to a Workspace; a default Workspace is
auto-created on session creation when not specified. Multiple sessions
may attach to the same Workspace.
**Alternatives considered:**
- Strict 1:1 with no sharing — rejected: blocks shared working state.
- Explicit-only attach (no defaults) — rejected: breaks existing clients.
**Rationale:** Defaulting preserves legacy ergonomics while opening the
multi-session sharing story for API clients.

### Decision 4: Text/Binary Encoding
**Chosen:** Automatic detection with base64 encoding for binary
**Rationale:** Optimize for the common case (text) while supporting binary.

### Decision 5: Workspace Mount Point
**Chosen:** Workspace files exposed to agents at `/workspace`
**Rationale:** `/workspace` is a common convention (VS Code DevContainers,
GitHub Codespaces). All capabilities normalize paths by stripping/adding
the `/workspace` prefix when interfacing with the workspace file store.

### Decision 6: Per-Workspace Storage Quotas
**Chosen:** Enforce byte-level quotas at the application layer in both
the HTTP API path and the agent tool path.

**Limits (env-configurable):**
- `WORKSPACE_FILE_MAX_BYTES` — total bytes per workspace (default 500 MB)
- `WORKSPACE_FILE_SINGLE_MAX_BYTES` — per-file ceiling (default 100 MB)

**Rationale:** Prevents unbounded PostgreSQL BYTEA growth from agentic file
writes (TM-FS-008 / TM-DOS-005).

## Requirements

### WorkspaceFile Model

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `workspace_id` | UUID v7 | Owning workspace reference |
| `path` | string | Absolute path starting with `/` |
| `content` | bytes? | File content (null for directories) |
| `is_directory` | bool | True for directories |
| `is_readonly` | bool | Prevents modification |
| `size_bytes` | i64 | Content size |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### Workspace Mount Point

Workspace files are exposed to agents at `/workspace`. Same translation as
before — agent paths use `/workspace/*` prefix, storage paths are normalized
internally.

### Path Validation

- Must start with `/`
- No null bytes
- No `..` path traversal
- No double slashes (`//`)
- Unique per workspace

### Behavior

1. **Auto-create parents:** Creating `/a/b/c.txt` automatically creates `/a` and `/a/b` directories.
2. **Delete cascade vs archive:** `DELETE /v1/workspaces/{id}` is a soft-delete that flips the workspace to `archived` and **keeps** its files (archived workspaces are read-only — the workspace fs HTTP layer rejects POST/PUT/DELETE on non-`active` workspaces). The `workspace_files.workspace_id` FK has `ON DELETE CASCADE`, so a hard row removal in `workspaces` (today only reachable through an out-of-band SQL deletion or a future hard-delete API) does cascade and removes all files.
3. **Encoding detection:** Files with null bytes in first 8KB are base64 encoded.
4. **Readonly protection:** Cannot modify or delete readonly files; recursive delete of a directory fails if it contains any readonly files.
5. **Hash-based freshness:** `read_file` and `write_file` return a `content_hash` (`sha256:...`). `edit_file` requires that hash and rejects stale edits.
6. **Text-only edit tool:** `edit_file` only operates on text files. Binary/base64 files must be replaced via `write_file`.
7. **Memory mounts:** Sessions can mount org Memories (`specs/memory.md`) into the workspace, read-only by default or read-write where trust permits.

### Database Schema

See `crates/server/migrations/056_workspaces.sql` for the schema.

### UI Integration

For the foundational PR (#PR2), no UI changes are made — the existing
session-detail "Workspace" tab continues to work via the session-aliased
routes. A dedicated Workspaces page is follow-up work.

## Git Version Control

Per-session git version control (current model) is preserved unchanged in
this PR. Git objects/refs remain keyed by `session_id`; promoting them to
workspace-scoping is follow-up work.

### Storage

- `session_git_objects` — session-scoped content-addressable store
- `session_git_refs` — session-scoped refdb
- See `crates/server/migrations/008_v0.8.7.sql` for DDL

### API Endpoints

All under `/v1/sessions/{session_id}/git/`. Unchanged.

## Open Questions

- **Workspace-scoped Git** — promote `session_git_*` to `workspace_git_*` so
  multi-session workspaces share one git history?
- **Workspace tables / KV / secrets** — promote `session_sqldb`,
  `session_key_values`, `session_secrets` to workspace-scoping?
- **Workspace UI** — dedicated listing/detail page, like Memory?
- **Per-workspace quotas in DB** — enforce `WORKSPACE_FILE_MAX_BYTES` via a
  DB trigger as defense in depth?
