---
type: Specification
title: "Memory Specification"
description: "Org-scoped named Memories (mountable into Workspaces)."
tags:
  - everruns
  - runtime-resources
---
# Memory Specification

## Abstract

Memories are persistent file stores mounted into session workspaces. The
default scope is org-wide and user-selected through the `memory` capability.
Everruns also has server-managed `agent` and `user` scoped memories for
private durable context that follows an agent or user across sessions.

This spec is the durable design intent for the Memory tier. The file surface
is the only surface implemented today (it descends from the earlier
"Workspace Volumes" concept). Future surfaces, tables, key-value, secrets,
structured, are tracked under [Open Questions](#open-questions).

## Model

Everruns has two memory tiers, distinguished by lifetime and scope:

* **Workspace** (`knowledge/runtime-resources/workspace.md`), the active working area for a
  session. Today: per-session, singleton, ephemeral by default. Future:
  may be shareable across sessions. Mounted at `/workspace`.
* **Memory** (this spec), org-scoped, named, durable, shared. Selected and
  mounted into a Workspace at session creation. RO by default; RW where
  trust permits.

A Memory is a **named addressable thing**. An org can have many
(`mem:crm`, `mem:legal`, `mem:runbooks`). Projects bundle multiple memories
or subtrees under a single mount prefix, bundles are out of scope for V1
but the mount API is shaped so they can be added without a breaking change.

## Concepts

| Name        | Description                                                       |
|-------------|-------------------------------------------------------------------|
| **Memory**  | Persistent file store. Public ID prefix: `mem_`.                 |
| **Org Memory** | User-managed shared store selected by the `memory` capability. |
| **Agent Memory** | Server-managed store owned by one agent and mounted at `/memory/agent` for sessions hosted by that agent. |
| **User Memory** | Server-managed store owned by one user and mounted at `/memory/user` for private one-user/default sessions. |
| **Manual Memory** | User-managed Memory whose files can be edited directly.     |
| **Source-backed Memory** | Provider-synced Memory populated from an external repository and exposed read-only. |
| **Memory File** | A file or directory entry inside a Memory.                    |
| **Mount**   | Binding of a Memory to a path under `/workspace` for a session.   |
| **Capability config** | `mounts[]` array on the `memory` capability.            |
| **Access mode** | `readonly` (default) or `readwrite`.                          |

## Lifecycle

Memories follow the standard building-block lifecycle from `knowledge/foundations/models.md`:

* `active`, assignable and editable.
* `archived`, read-only, hidden from default lists, not assignable to new mounts.
* `deleted`, tombstone; detail/list APIs return 404 except for historical references.

## Data Model

### `memories`

| Column                    | Type        | Notes                                                  |
|---------------------------|-------------|--------------------------------------------------------|
| `id`                      | UUID PK     | Internal primary key.                                  |
| `org_id`                  | BIGINT FK   | Organization scope.                                    |
| `public_id`               | TEXT        | `mem_<32-hex>`. Unique per `org_id`.                   |
| `name`                    | VARCHAR     | Unique within `org_id` while not deleted.              |
| `description`             | TEXT?       |                                                        |
| `scope`                   | VARCHAR     | `org` / `agent` / `user`. Defaults to `org`.           |
| `owner_agent_id`          | UUID?       | Required for `scope = agent`; otherwise NULL.          |
| `owner_user_id`           | UUID?       | Required for `scope = user`; otherwise NULL.           |
| `source_type`             | VARCHAR     | `manual` / `github` / `git`. Defaults to `manual`.     |
| `source_config`           | JSONB       | Non-secret source coordinates. `{}` for manual.        |
| `is_readonly`             | BOOL        | True for source-backed Memories.                       |
| `sync_status`             | VARCHAR     | `idle` / `pending` / `syncing` / `synced` / `failed`.  |
| `last_synced_at`          | TIMESTAMPTZ? | Last successful provider sync.                        |
| `last_sync_error`         | TEXT?       | Sanitized last sync error for UI/admin debugging.      |
| `owner_principal_id`      | TEXT?       | Principal that created the memory.                     |
| `resolved_owner_user_id`  | UUID?       | Resolved user id, if known.                            |
| `status`                  | VARCHAR     | `active` / `archived` / `deleted`.                     |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                                       |
| `archived_at` / `deleted_at` | TIMESTAMPTZ? |                                                     |

`UNIQUE(org_id, public_id)` and `UNIQUE(org_id, name) WHERE status != 'deleted'`.
Scoped memories additionally enforce one active `agent` memory per
`(org_id, owner_agent_id)` and one active `user` memory per
`(org_id, owner_user_id)`. Non-org scoped memories are server-managed manual,
read-write stores; source-backed sync is only user-facing for org memories.

`source_config` never stores provider credentials. GitHub sources use the
creator's GitHub connection or a future agent identity connection at sync time.
Generic Git sources are limited to public URLs or credential-less SSH/HTTPS
coordinates until a credential reference type exists.

GitHub source example:

```json
{
  "provider": "github",
  "repository": "everruns/everruns",
  "branch": "main",
  "root_folder": "specs"
}
```

Generic Git source example:

```json
{
  "provider": "git",
  "url": "https://github.com/everruns/everruns.git",
  "branch": "main",
  "root_folder": null
}
```

### `memory_files`

Mirrors the Workspace file shape. Path validation matches workspace file
validation: starts with `/`, no null bytes, no `..`, no `//`, unique per
`(memory_id, path)`.

| Column          | Type        | Notes                                |
|-----------------|-------------|--------------------------------------|
| `id`            | UUID PK     |                                      |
| `memory_id`     | UUID FK     | `ON DELETE CASCADE`.                 |
| `path`          | TEXT        | Absolute, normalized.                |
| `content`       | BYTEA?      | NULL for directories.                |
| `is_directory`  | BOOL        |                                      |
| `size_bytes`    | BIGINT      |                                      |
| `content_hash`  | TEXT?       | `sha256:...` for files.              |
| `created_at` / `updated_at` | TIMESTAMPTZ |                          |

### `session_memory_mounts`

Snapshot of mount configuration at session creation time. Runtime behavior is
stable even if the agent or harness config changes after session start, and
memory archival/deletion is handled gracefully against this snapshot.

| Column          | Type      | Notes                                       |
|-----------------|-----------|---------------------------------------------|
| `id`            | UUID PK   |                                             |
| `session_id`    | UUID FK   | `ON DELETE CASCADE`.                        |
| `memory_id`     | UUID FK   | Snapshot, survives memory archive/delete.  |
| `mount_path`    | TEXT      | Normalized under `/workspace`.              |
| `access`        | VARCHAR   | `readonly` / `readwrite`.                   |
| `created_at`    | TIMESTAMPTZ |                                           |

## Capability: `memory`

* **ID:** `memory`
* **Name:** `Memory`
* **Category:** `Memory`
* **Icon:** `brain`
* **Dependencies:** `workspace`
* **Features:** `file_system`
* **Risk:** `Medium`, shared writeable mounts can let one session influence
  future sessions.

### Config Schema

```json
{
  "mounts": [
    {
      "memory": "mem_abc123...",
      "path": "/workspace/research",
      "mode": "readonly"
    },
    {
      "memory": "mem_def456...",
      "path": "/workspace/team-notes",
      "mode": "readwrite"
    }
  ]
}
```

### Validation Rules

* `memory` must reference an `active` Memory in the current org. Cross-org
  references are rejected without leaking existence of other-org Memories.
* The public `memory` capability can mount only org-scoped Memories. Agent and
  user scoped Memories are mounted automatically by the server.
* `path` must normalize under `/workspace`.
* `/memory/*` is reserved for server-managed scoped memory and cannot be used
  by caller-supplied initial files or capability mounts.
* `mode` defaults to `readonly` when omitted.
* Reject duplicate mount paths.
* Reject overlapping mount paths in V1 (one mount path cannot be a prefix of
  another).
* Reject mounts at reserved system paths and at the roots of existing
  capability mounts.

## Runtime Mount Semantics

### Scoped Memory Mounts

When creating a session, the server lazily creates missing scoped Memories and
mounts their current file tree into the session workspace:

* Host agent memory: `/memory/agent`, read-write, one Memory per active agent.
* User memory: `/memory/user`, read-write, one Memory per user. V1 mounts this
  only into the default one-session workspace where the workspace is private to
  that session owner. It is intentionally not materialized into caller-attached
  shared workspaces until runtime mounts are participant-local rather than
  workspace-wide.

Guest agent memory follows the same ownership model, but fully isolated
`/memory/agent` paths for multiple agent participants in one shared workspace
require a participant-local runtime mount context. Until that exists, the
server must not merge another agent's private memory into a shared
workspace-wide `/memory/agent` path.

### Read

* `read_file`, `list_directory`, `grep_files`, `stat_file`, the Workspace UI,
  direct worker adapters, and `bashkit_shell` include mounted Memory files.
* Path display remains under `/workspace`; storage paths are normalized
  internally.
* Directory listings merge workspace files and mounted Memory files
  deterministically (mount overlay first, workspace file second when paths
  collide; collisions are rejected at session creation, so this only matters
  for new files written under the mount root).

### Write

* Source-backed Memories are read-only regardless of mount access. Capability
  validation must reject `readwrite` mounts for `source_type != 'manual'`.
  Direct Memory filesystem write APIs must return a read-only error.
* Read-only mounted paths reject `write_file`, `edit_file`, `delete_file`,
  moves into the mount root, and copies that overwrite mounted content.
* Read-write mounted paths write through to `memory_files`. The Memory row's
  `updated_at` is refreshed.
* Stale-edit protection uses `content_hash` exactly like workspace files.
* Writes outside mounted paths continue to use workspace-local files.

## Source Sync

Source-backed Memories populate `memory_files` by syncing an external
repository into the same storage shape used by manual Memories. Consumers
read them through the regular Memory mount and filesystem APIs; no
source-specific read tool is introduced.

### GitHub

Creation accepts:

```json
{
  "source": {
    "type": "github",
    "repository": "owner/repo",
    "branch": "main",
    "root_folder": "docs"
  }
}
```

`repository` may be `owner/repo` or a `github.com` repository URL. `branch`
defaults to `main`; `root_folder` is optional and normalized as a relative path.
Private repository access uses the existing GitHub user connection. If no
connection is available or the installation cannot access the repository, sync
sets `sync_status = failed` with a sanitized error and leaves previous files in
place.

### Generic Git

Creation accepts:

```json
{
  "source": {
    "type": "git",
    "url": "https://example.com/org/repo.git",
    "branch": "main",
    "root_folder": "src"
  }
}
```

Generic Git URLs must not contain inline credentials. Credentialed generic Git
sync should use a future connection reference rather than embedding secrets in
the Memory row.

### Sync Semantics

* Source-backed creation sets `sync_status = pending`; a background sync worker
  claims pending/stale rows.
* Sync checks out the configured branch/ref, snapshots `root_folder`, and
  replaces the Memory file tree atomically so sessions never observe a partial
  update.
* Sync excludes VCS metadata (`.git/`) and applies the same path validation used
  by `memory_files`.
* If a sync fails, existing files remain readable, `sync_status = failed`, and
  `last_sync_error` stores a sanitized operator-facing reason.
* Consumers always see source-backed Memories as read-only regular Memories.

### Mount lifecycle vs Memory lifecycle

* If a mounted Memory is **archived** after session creation, existing reads
  continue but writes fail with a `memory_archived` error.
* If a mounted Memory is **deleted** after session creation, file access
  fails with a `memory_deleted` error and the UI surfaces the missing mount.

## API

REST endpoints (see `knowledge/execution/apis.md` for the full list, conventions, and
OpenAPI exposure):

* `GET    /v1/memories`
* `POST   /v1/memories`
* `GET    /v1/memories/{memory_id}`
* `PATCH  /v1/memories/{memory_id}`
* `DELETE /v1/memories/{memory_id}`, archive / delete per lifecycle
* `GET    /v1/memories/{memory_id}/fs/...`
* `POST   /v1/memories/{memory_id}/fs/...`
* `PUT    /v1/memories/{memory_id}/fs/...`
* `DELETE /v1/memories/{memory_id}/fs/...`
* `POST   /v1/memories/{memory_id}/fs/_/{stat,grep}`
* `GET    /v1/memories/{memory_id}/fs/_/download/{path}`

`move` and `copy` actions are deferred, the spec leaves room for them but
the file CRUD ships first; clients can compose them with create + delete.
Filesystem sub-routes mirror `knowledge/runtime-resources/workspace.md` request/response shapes so
DTOs and UI components can be reused.

## UI

* Top-level **Memory** page, list, search, archive toggle, create button.
* **Memory detail** page, editable name/description, file browser/editor
  reusing the Workspace tab components, archive/delete actions, and a usage
  panel showing agents/harnesses/sessions currently mounting the Memory.
* **Capability config UI** for `memory`, add/remove mount rows with
  Memory selector, mount path input, access mode selector, and inline
  validation for duplicate/overlapping paths and archived/deleted Memories.
* **Session Workspace UI**: mounted files render with a `Memory` badge plus
  a `Read-only` or `Read-write` mode badge; tooltip shows source Memory name
  and mount path; read-only write attempts surface a clear error.

## Security

See `knowledge/security/threat-model.md` for the canonical entries.

* **Cross-org reference:** Mount config validation MUST reject Memory IDs from
  other orgs. Errors must not leak existence.
* **Path traversal:** Memory file path validation mirrors workspace file
  validation exactly.
* **Reserved paths:** Mounts cannot shadow workspace system paths (e.g.
  `/.agents/`, `/outputs/`).
* **Read-write trust boundary:** Read-write mounts on shared agents/harnesses
  let one session influence future sessions. Treat as a meaningful trust
  boundary; capability risk level is `Medium`.
* **Audit:** Memory CRUD (create, update, archive, delete) and mount
  configuration changes are audited via the standard audit log domain.

## Permissions

Memory CRUD is org-scoped. Standard org policies (`org.member`, `org.admin`)
apply. Mount configuration is gated by the same policy that gates capability
configuration on the parent agent or harness.

## Testing

Required coverage:

* Memory ID parsing/serialization.
* Memory path validation mirrors workspace file validation.
* Mount config parsing defaults `mode` to `readonly`.
* Reject overlapping mount paths.
* Reject cross-org / archived / deleted Memory references.
* Read-only write attempts return readonly errors.
* Read-write writes update Memory store.
* Source-backed Memories cannot be mounted read-write.
* Source-backed Memory filesystem write APIs return readonly errors.
* GitHub/git source creation validates repository/ref/root-folder shape.
* Source sync atomically replaces `memory_files` and preserves previous files
  on sync failure.
* Content-hash stale edits fail on read-write mounts.
* Directory listing merge order is deterministic.
* Grep searches mounted content and workspace-local content.
* Storage parity (in-memory and Postgres) for `memories`, `memory_files`,
  `session_memory_mounts`.
* API CRUD permissions and org scoping.
* OpenAPI export updated when API surface changes.
* UI list/detail/capability-config/Workspace badge flows.
* Manual test cases (`test_cases/memory/`).

## Open Questions

* **Additional surfaces.** Today a Memory has a single file surface. The
  durable intent is to add tabular, key-value, secrets, and structured
  (knowledge-base-like) surfaces under the same Memory entity, addressed as
  `mem:<name>/<surface>/...`. Order and timing TBD.
* **Project bundles.** A Project bundles multiple memories or subtrees under
  one mount prefix. Not in V1; mount API is shaped to accept bundles later.
* Should read-write mounts require admin role when configured on shared
  agents/harnesses?
* Should archived Memories remain readable for existing sessions, or become
  inaccessible immediately?
* Should V1 track Memory file version history, or defer to a future versioned
  filesystem feature?
* Should mount conflicts always fail, or should users be able to choose
  precedence later?
* What sync cadence should source-backed Memories use after the initial sync:
  webhook-triggered, periodic polling, or both?
* Should source-backed Memories bind to creator user connections, agent
  identity connections, or explicit per-Memory connection references?
