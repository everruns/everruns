# Session Virtual Filesystem Specification

## Abstract

This document defines the session-level virtual filesystem for Everruns. Each session has an isolated filesystem backed by PostgreSQL, enabling agents to read, write, and manipulate files during execution.

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
5. **Capability mounts:** When a session is created, files from capability mount points are automatically populated (see `specs/capabilities.md` for details)

### Database Schema

See `crates/server/migrations/001_base_schema.sql` for the `session_files` table DDL (includes path validation, unique constraints, and indexes).

### UI Integration

- "Workspace" tab on session detail page
- File browser component with directory navigation
- File viewer with Preview/Source toggle
- File preview support:
  - **Code files** (ts, js, py, rs, go, etc.): Shiki-based syntax highlighting via Streamdown
  - **CSV files**: Rendered as formatted tables with row/column count
  - **JSON files**: Pretty-printed with Shiki syntax highlighting
  - **Markdown files**: Full rendering with GFM, code blocks, alerts
  - **Images** (png, jpg, gif, webp, svg): Inline preview
- File editor with save functionality (text files only)
- Create file/folder dialogs
- Delete confirmation
- Download file support
