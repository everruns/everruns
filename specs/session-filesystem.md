# Session Virtual Filesystem Specification

## Abstract

This document defines the session-level virtual filesystem for Everruns. Each session has an isolated filesystem backed by PostgreSQL, enabling agents to read, write, edit, and manipulate files during execution.

## Design Decisions

### Decision 1: PostgreSQL-backed Storage
**Chosen:** Store files in PostgreSQL BYTEA column
**Alternatives considered:**
- Object storage (S3, MinIO): Added complexity for MVP
- Local filesystem: Not suitable for distributed deployments
**Rationale:** PostgreSQL provides ACID transactions, simple deployment, and good performance for small-to-medium files. Can migrate to object storage later for large files.

### Decision 2: RESTful Path-based API
**Chosen:** `/fs/{path}` with HTTP methods mapping to operations
**Alternatives considered:**
- Query parameter based (`/files?path=/foo.txt`)
- Action-based (`/files/read`, `/files/write`)
**Rationale:** Path-based URLs are more RESTful and intuitive. GET reads, POST creates, PUT updates, DELETE removes.

### Decision 3: Session Isolation
**Chosen:** Each session has its own isolated filesystem
**Alternatives considered:**
- Agent-level filesystem (shared across sessions)
- Global filesystem (shared across agents)
**Rationale:** Session isolation provides better security and prevents interference between concurrent sessions. Agents can use different files per conversation context.

### Decision 4: Text/Binary Encoding
**Chosen:** Automatic detection with base64 encoding for binary
**Alternatives considered:**
- Always base64: Wastes bandwidth for text
- Multipart upload: More complex API
**Rationale:** Text files are common, so optimize for them. Use base64 only when necessary (null bytes detected).

### Decision 5: Out of Scope - Large Files
**Chosen:** No special handling for large files (>10MB)
**Rationale:** MVP focuses on code and config files. Large file streaming can be added later with object storage backend.

### Decision 6: Workspace Mount Point
**Chosen:** Session files exposed to agents at `/workspace`
**Alternatives considered:**
- Mount at root `/`: Conflicts with system directories
- Mount at `/home/agent`: Confusing with bash HOME directory
- Mount at `/app/session`: Less intuitive
**Rationale:** `/workspace` is a common convention (similar to VS Code DevContainers, GitHub Codespaces) and clearly indicates agent work area. All capabilities (file_system, virtual_bash) normalize paths by stripping/adding the `/workspace` prefix when interfacing with the session file store.

## Requirements

### SessionFile Model

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `path` | string | Absolute path starting with `/` |
| `content` | bytes? | File content (null for directories) |
| `is_directory` | bool | True for directories |
| `is_readonly` | bool | Prevents modification |
| `size_bytes` | i64 | Content size |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### Workspace Mount Point

Session files are exposed to agents at `/workspace`:

- **Agent view**: Files appear at `/workspace/*` (e.g., `/workspace/src/main.rs`)
- **Store view**: Files stored with normalized paths (e.g., `/src/main.rs`)
- **Path translation**: Capabilities strip `/workspace` prefix before storage, add it back for display

This enables both the file_system capability (tools) and virtual_bash capability to share the same file store seamlessly.

### Path Validation

- Must start with `/`
- No null bytes
- No `..` path traversal
- No double slashes (`//`)
- Unique per session

### API Endpoints

All endpoints under `/v1/sessions/{session_id}/fs`

#### CRUD Operations

| Method | Path | Description |
|--------|------|-------------|
| GET | `/fs` | List root directory |
| GET | `/fs/{path}` | Read file or list directory |
| POST | `/fs/{path}` | Create file or directory |
| PUT | `/fs/{path}` | Update file content |
| DELETE | `/fs/{path}` | Delete file |
| DELETE | `/fs/{path}?recursive=true` | Delete directory recursively |

#### Actions

| Method | Path | Description |
|--------|------|-------------|
| POST | `/fs/_/stat` | Get file metadata |
| POST | `/fs/_/move` | Move/rename file |
| POST | `/fs/_/copy` | Copy file |
| POST | `/fs/_/grep` | Search files by content |

**Note:** Paths starting with `_` are reserved for system actions and cannot be used for file creation or updates.

### Request/Response

See the OpenAPI spec (`./scripts/export-openapi.sh`) for detailed request/response schemas. Key patterns:
- Create: `POST /fs/{path}` with `{ "content": "...", "encoding": "text" }`
- Directory: `POST /fs/{path}` with `{ "is_directory": true }`
- Grep: `POST /fs/_/grep` with `{ "pattern": "...", "path_pattern": "*.rs" }`

### Behavior

1. **Auto-create parents:** Creating `/a/b/c.txt` automatically creates `/a` and `/a/b` directories
2. **Delete cascade:** Deleting a session deletes all its files (via FK cascade)
3. **Encoding detection:** Files with null bytes in first 8KB are base64 encoded
4. **Readonly protection:** Cannot modify or delete readonly files; recursive delete of a directory fails if it contains any readonly files
5. **Hash-based freshness:** `read_file` and `write_file` return a `content_hash` (`sha256:...`). `edit_file` requires that hash and rejects stale edits, including writes that race after the initial read.
6. **Text-only edit tool:** `edit_file` only operates on text files. Binary/base64 files must be replaced via `write_file`.
7. **Exact replacement semantics:** `edit_file` supports single or batched exact replacements within one file. All replacements are matched against the original file content; ambiguous or overlapping matches are rejected.
8. **Formatting preservation:** `edit_file` preserves UTF-8 BOM and the file's existing newline convention (`LF`, `CRLF`, or `CR`).
9. **Bounded diff payloads:** `edit_file` returns a unified diff for transcript/UI rendering, but large diffs are truncated and flagged to avoid oversized tool payloads.
10. **Capability mounts:** When a session is created, files from capability mount points are automatically populated. Inline mounts are written to the database; virtual mounts (`MountSource::Virtual`) are registered in an in-memory `VirtualMountRegistry` and served without DB writes. See `specs/capabilities.md` for details.
11. **Starter files:** Agents and harnesses can declare starter files that are copied into each new session before use. Agent starter files override harness starter files when they target the same normalized path.

### Database Schema

See `crates/server/migrations/001_base_schema.sql` for the `session_files` table DDL (includes path validation, unique constraints, and indexes).

### UI Integration

- "Workspace" tab on session detail page
- File browser component with directory navigation
- File viewer with Preview/Source toggle
- Read-only starter files are visible in the UI but cannot be edited or deleted
- File preview support:
  - **Code files** (ts, js, py, rs, go, etc.): Shiki-based syntax highlighting via Streamdown
  - **CSV files**: Rendered as formatted tables with row/column count
  - **JSON files**: Pretty-printed with Shiki syntax highlighting
  - **Markdown files**: Full rendering with GFM, code blocks, alerts
  - **Images** (png, jpg, gif, webp, svg): Inline preview
- File editor with save functionality (text files only, should use freshness-checked edits)
- Create file/folder dialogs
- Delete confirmation
- Download file support

## Git Version Control

Per-session git version control backed by libgit2 with a PostgreSQL object/ref store.

### Design: Mempack Hydrate/Drain

Git operations use an in-memory libgit2 repository via the mempack backend:
1. **Load** all git objects + refs from PostgreSQL (single query each)
2. **Hydrate** a mempack ODB, perform git operations in `spawn_blocking`
3. **Drain** new objects + updated refs back to PostgreSQL

This avoids filesystem I/O entirely — the git repository exists only in memory during operations.

### Storage

- `session_git_objects` — session-scoped content-addressable store (session_id, 20-byte SHA1 OID, type, data)
- `session_git_refs` — session-scoped refdb (session_id, name, target OID)
- CHECK constraints enforce OID length (20 bytes) and object type (1–4)
- See `crates/server/migrations/008_v0.8.7.sql` for DDL

### API Endpoints

All under `/v1/sessions/{session_id}/git/`:

| Method | Path | Description |
|--------|------|-------------|
| POST | `commit` | Commit current session files |
| GET | `log` | Commit history (optional `ref`, `limit` params) |
| GET | `diff` | Diff between commits (`oid` required, `base` optional) |
| GET | `refs` | List all refs/branches |
| POST | `branches` | Create a branch |
| DELETE | `branches/{name}` | Delete a branch |

Branch names are normalized: short names like `"main"` become `refs/heads/main`.

### Implementation

- Service: `crates/server/src/services/session_git.rs`
- API: `crates/server/src/api/session_git.rs`
- Storage: `crates/server/src/storage/{memory,repositories}/session_git.rs`
