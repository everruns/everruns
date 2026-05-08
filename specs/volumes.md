# Workspace Volumes Specification

## Abstract

Volumes are org-scoped, named, persistent filesystem trees that users mount into
session workspaces through the `workspace_volumes` capability. Volumes provide
shared, file-backed memory and durable data across sessions while reusing the
existing `/workspace` session filesystem tools and UI patterns.

This spec is the durable design intent for [EVE-396]. Implementation is delivered
across multiple PRs; the entity, ID schema, capability registration, and DB
schema land in the foundational PR. CRUD APIs, filesystem APIs, mount resolution,
and UI follow as additional vertical slices.

## Motivation

Everruns has three adjacent primitives, but none cover shared durable files:

* **Session filesystem** (`session_files`) — isolated per session under
  `/workspace`; not shared after creation.
* **Capability mounts** — `MountPoint` / `MountSource` materialize starter
  files from capability code at session creation, but they are not user-managed
  shared state.
* **Persistent memory** — cross-session semantic facts; not suitable for
  structured files, datasets, artifacts, or project folders.

Users need a way to maintain reusable file collections and attach them to
agents, harnesses, or sessions either as read-only reference data or as
read-write shared working memory.

## Concepts

| Name        | Description                                                       |
|-------------|-------------------------------------------------------------------|
| **Volume**  | Org-scoped, named filesystem tree. Public ID prefix: `vol_`.      |
| **Manual Volume** | User-managed Volume whose files can be edited directly.      |
| **Source-backed Volume** | Provider-synced Volume populated from an external repository and exposed read-only. |
| **Volume File** | A file or directory entry inside a Volume.                    |
| **Mount**   | Binding of a Volume to a path under `/workspace` for a session.   |
| **Capability config** | `mounts[]` array on `workspace_volumes` capability.     |
| **Access mode** | `readonly` (default) or `readwrite`.                          |

## Lifecycle

Volumes follow the standard building-block lifecycle from `specs/models.md`:

* `active` — assignable and editable.
* `archived` — read-only, hidden from default lists, not assignable to new mounts.
* `deleted` — tombstone; detail/list APIs return 404 except for historical references.

## Data Model

### `volumes`

| Column                    | Type        | Notes                                                  |
|---------------------------|-------------|--------------------------------------------------------|
| `id`                      | UUID PK     | Internal primary key.                                  |
| `org_id`                  | BIGINT FK   | Organization scope.                                    |
| `public_id`               | TEXT        | `vol_<32-hex>`. Unique per `org_id`.                   |
| `name`                    | VARCHAR     | Unique within `org_id` while not deleted.              |
| `description`             | TEXT?       |                                                        |
| `source_type`             | VARCHAR     | `manual` / `github` / `git`. Defaults to `manual`.     |
| `source_config`           | JSONB       | Non-secret source coordinates. `{}` for manual.        |
| `is_readonly`             | BOOL        | True for source-backed Volumes.                        |
| `sync_status`             | VARCHAR     | `idle` / `pending` / `syncing` / `synced` / `failed`.  |
| `last_synced_at`          | TIMESTAMPTZ? | Last successful provider sync.                        |
| `last_sync_error`         | TEXT?       | Sanitized last sync error for UI/admin debugging.      |
| `owner_principal_id`      | TEXT?       | Principal that created the volume.                     |
| `resolved_owner_user_id`  | UUID?       | Resolved user id, if known.                            |
| `status`                  | VARCHAR     | `active` / `archived` / `deleted`.                     |
| `created_at` / `updated_at` | TIMESTAMPTZ |                                                       |
| `archived_at` / `deleted_at` | TIMESTAMPTZ? |                                                     |

`UNIQUE(org_id, public_id)` and `UNIQUE(org_id, name) WHERE status != 'deleted'`.

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

### `volume_files`

Mirrors `session_files` shape. Path validation matches session filesystem
validation: starts with `/`, no null bytes, no `..`, no `//`, unique per
`(volume_id, path)`.

| Column          | Type        | Notes                                |
|-----------------|-------------|--------------------------------------|
| `id`            | UUID PK     |                                      |
| `volume_id`     | UUID FK     | `ON DELETE CASCADE`.                 |
| `path`          | TEXT        | Absolute, normalized.                |
| `content`       | BYTEA?      | NULL for directories.                |
| `is_directory`  | BOOL        |                                      |
| `size_bytes`    | BIGINT      |                                      |
| `content_hash`  | TEXT?       | `sha256:...` for files.              |
| `created_at` / `updated_at` | TIMESTAMPTZ |                          |

### `session_volume_mounts`

Snapshot of mount configuration at session creation time. Runtime behavior is
stable even if the agent or harness config changes after session start, and
volume archival/deletion is handled gracefully against this snapshot.

| Column          | Type      | Notes                                       |
|-----------------|-----------|---------------------------------------------|
| `id`            | UUID PK   |                                             |
| `session_id`    | UUID FK   | `ON DELETE CASCADE`.                        |
| `volume_id`     | UUID FK   | Snapshot — survives volume archive/delete.  |
| `mount_path`    | TEXT      | Normalized under `/workspace`.              |
| `access`        | VARCHAR   | `readonly` / `readwrite`.                   |
| `created_at`    | TIMESTAMPTZ |                                           |

## Capability: `workspace_volumes`

* **ID:** `workspace_volumes`
* **Name:** `Workspace Volumes`
* **Category:** `File System`
* **Icon:** `hard-drive`
* **Dependencies:** `session_file_system`
* **Features:** `file_system`
* **Risk:** `Medium` — shared writeable mounts can let one session influence
  future sessions.

### Config Schema

```json
{
  "mounts": [
    {
      "volume": "vol_abc123...",
      "path": "/workspace/research",
      "mode": "readonly"
    },
    {
      "volume": "vol_def456...",
      "path": "/workspace/team-memory",
      "mode": "readwrite"
    }
  ]
}
```

### Validation Rules

* `volume` must reference an `active` Volume in the current org. Cross-org
  references are rejected without leaking existence of other-org Volumes.
* `path` must normalize under `/workspace`.
* `mode` defaults to `readonly` when omitted.
* Reject duplicate mount paths.
* Reject overlapping mount paths in V1 (one mount path cannot be a prefix of
  another).
* Reject mounts at reserved system paths and at the roots of existing
  capability mounts.

## Runtime Mount Semantics

### Read

* `read_file`, `list_directory`, `grep_files`, `stat_file`, the Workspace UI,
  direct worker adapters, and `virtual_bash` include mounted Volume files.
* Path display remains under `/workspace`; storage paths are normalized
  internally.
* Directory listings merge session files and mounted Volume files
  deterministically (mount overlay first, session file second when paths
  collide; collisions are rejected at session creation, so this only matters
  for new files written under the mount root).

### Write

* Source-backed Volumes are read-only regardless of mount access. Capability
  validation must reject `readwrite` mounts for `source_type != 'manual'`.
  Direct Volume filesystem write APIs must return a read-only error.
* Read-only mounted paths reject `write_file`, `edit_file`, `delete_file`,
  moves into the mount root, and copies that overwrite mounted content.
* Read-write mounted paths write through to `volume_files`. The Volume row's
  `updated_at` is refreshed.
* Stale-edit protection uses `content_hash` exactly like `session_files`.
* Writes outside mounted paths continue to use session-local files.

## Source Sync

Source-backed Volumes populate `volume_files` by syncing an external repository
into the same storage shape used by manual Volumes. Consumers read them through
the regular Volume mount and filesystem APIs; no source-specific read tool is
introduced.

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
the Volume row.

### Sync Semantics

* Source-backed creation sets `sync_status = pending`; a background sync worker
  claims pending/stale rows.
* Sync checks out the configured branch/ref, snapshots `root_folder`, and
  replaces the Volume file tree atomically so sessions never observe a partial
  update.
* Sync excludes VCS metadata (`.git/`) and applies the same path validation used
  by `volume_files`.
* If a sync fails, existing files remain readable, `sync_status = failed`, and
  `last_sync_error` stores a sanitized operator-facing reason.
* Consumers always see source-backed Volumes as read-only regular Volumes.

### Mount lifecycle vs Volume lifecycle

* If a mounted Volume is **archived** after session creation, existing reads
  continue but writes fail with a `volume_archived` error.
* If a mounted Volume is **deleted** after session creation, file access
  fails with a `volume_deleted` error and the UI surfaces the missing mount.

## API

REST endpoints (see `specs/apis.md` for the full list, conventions, and
OpenAPI exposure):

* `GET    /v1/volumes`
* `POST   /v1/volumes`
* `GET    /v1/volumes/{volume_id}`
* `PATCH  /v1/volumes/{volume_id}`
* `DELETE /v1/volumes/{volume_id}` — archive / delete per lifecycle
* `GET    /v1/volumes/{volume_id}/fs/...`
* `POST   /v1/volumes/{volume_id}/fs/...`
* `PUT    /v1/volumes/{volume_id}/fs/...`
* `DELETE /v1/volumes/{volume_id}/fs/...`
* `POST   /v1/volumes/{volume_id}/fs/_/{stat,move,copy,grep}`
* `GET    /v1/volumes/{volume_id}/fs/_/download/{path}`

Filesystem sub-routes mirror `specs/session-filesystem.md` request/response
shapes so DTOs and UI components can be reused.

## UI

* Top-level **Volumes** page — list, search, archive toggle, create button.
* **Volume detail** page — editable name/description, file browser/editor
  reusing the session Workspace tab components, archive/delete actions, and
  a usage panel showing agents/harnesses/sessions currently mounting the
  Volume.
* **Capability config UI** for `workspace_volumes` — add/remove mount rows
  with Volume selector, mount path input, access mode selector, and inline
  validation for duplicate/overlapping paths and archived/deleted Volumes.
* **Session Workspace UI** — mounted files render with a `Volume` badge plus
  a `Read-only` or `Read-write` mode badge; tooltip shows source Volume name
  and mount path; read-only write attempts surface a clear error.

## Security

See `specs/threat-model.md` for the canonical entries.

* **Cross-org reference:** Mount config validation MUST reject Volume IDs from
  other orgs. Errors must not leak existence.
* **Path traversal:** Volume file path validation mirrors session file
  validation exactly.
* **Reserved paths:** Mounts cannot shadow session system paths (e.g. `/.agents/`,
  `/.outputs/`).
* **Read-write trust boundary:** Read-write mounts on shared agents/harnesses
  let one session influence future sessions. Treat as a meaningful trust
  boundary; capability risk level is `Medium`.
* **Audit:** Volume CRUD (create, update, archive, delete) and mount
  configuration changes are audited via the standard audit log domain.

## Permissions

Volume CRUD is org-scoped. Standard org policies (`org.member`, `org.admin`)
apply. Mount configuration is gated by the same policy that gates capability
configuration on the parent agent or harness.

## Testing

See the per-PR slice notes in [EVE-396] for the exact test sets. Required
coverage at full delivery:

* Volume ID parsing/serialization.
* Volume path validation mirrors session file validation.
* Mount config parsing defaults `mode` to `readonly`.
* Reject overlapping mount paths.
* Reject cross-org / archived / deleted Volume references.
* Read-only write attempts return readonly errors.
* Read-write writes update Volume store.
* Source-backed Volumes cannot be mounted read-write.
* Source-backed Volume filesystem write APIs return readonly errors.
* GitHub/git source creation validates repository/ref/root-folder shape.
* Source sync atomically replaces `volume_files` and preserves previous files on
  sync failure.
* Content-hash stale edits fail on read-write mounts.
* Directory listing merge order is deterministic.
* Grep searches mounted content and session-local content.
* Storage parity (in-memory and Postgres) for `volumes`, `volume_files`,
  `session_volume_mounts`.
* API CRUD permissions and org scoping.
* OpenAPI export updated when API surface changes.
* UI list/detail/capability-config/Workspace badge flows.
* Manual test cases (`test_cases/volumes/`).

## Open Questions

* Should read-write mounts require admin role when configured on shared
  agents/harnesses? (Tracked in [EVE-396] open questions.)
* Should archived Volumes remain readable for existing sessions, or become
  inaccessible immediately?
* Should V1 track Volume file version history, or defer to a future versioned
  filesystem feature?
* Should mount conflicts always fail, or should users be able to choose
  precedence later?
* What sync cadence should source-backed Volumes use after the initial sync:
  webhook-triggered, periodic polling, or both?
* Should source-backed Volumes bind to creator user connections, agent identity
  connections, or explicit per-Volume connection references?

[EVE-396]: https://linear.app/everruns/issue/EVE-396
