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

### Path Validation

- Must start with `/`
- No null bytes
- No `..` path traversal
- No double slashes (`//`)
- Unique per session

### API Endpoints

All endpoints under `/v1/agents/{agent_id}/sessions/{session_id}/fs`

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

### Request/Response Examples

**Create File:**
```json
POST /v1/agents/{id}/sessions/{id}/fs/src/main.rs
{
  "content": "fn main() {}",
  "encoding": "text"
}
```

Response:
```json
{
  "id": "...",
  "session_id": "...",
  "path": "/src/main.rs",
  "name": "main.rs",
  "content": "fn main() {}",
  "encoding": "text",
  "is_directory": false,
  "is_readonly": false,
  "size_bytes": 12,
  "created_at": "...",
  "updated_at": "..."
}
```

**Create Directory:**
```json
POST /v1/agents/{id}/sessions/{id}/fs/docs
{
  "is_directory": true
}
```

**List Directory:**
```json
GET /v1/agents/{id}/sessions/{id}/fs/src
{
  "data": [
    {
      "id": "...",
      "path": "/src/main.rs",
      "name": "main.rs",
      "is_directory": false,
      "size_bytes": 12,
      ...
    }
  ]
}
```

**Grep Search:**
```json
POST /v1/agents/{id}/sessions/{id}/fs/_actions/grep
{
  "pattern": "fn\\s+\\w+",
  "path_pattern": "*.rs"
}
```

Response:
```json
{
  "data": [
    {
      "path": "/src/main.rs",
      "matches": [
        {
          "path": "/src/main.rs",
          "line_number": 1,
          "line": "fn main() {}"
        }
      ]
    }
  ]
}
```

### Behavior

1. **Auto-create parents:** Creating `/a/b/c.txt` automatically creates `/a` and `/a/b` directories
2. **Delete cascade:** Deleting a session deletes all its files (via FK cascade)
3. **Encoding detection:** Files with null bytes in first 8KB are base64 encoded
4. **Readonly protection:** Cannot modify content of readonly files (can still delete)

### Database Schema

```sql
CREATE TABLE session_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content BYTEA,
    is_directory BOOLEAN NOT NULL DEFAULT FALSE,
    is_readonly BOOLEAN NOT NULL DEFAULT FALSE,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT session_files_unique_path UNIQUE (session_id, path),
    CONSTRAINT session_files_path_check CHECK (path ~ '^/([^/\0]+(/[^/\0]+)*)?$'),
    CONSTRAINT session_files_directory_no_content CHECK (NOT is_directory OR content IS NULL)
);

CREATE INDEX session_files_session_idx ON session_files(session_id);
CREATE INDEX session_files_path_prefix_idx ON session_files(session_id, path text_pattern_ops);
```

### UI Integration

- "File System" tab on session detail page
- File browser component with directory navigation
- File viewer/editor with save functionality
- Create file/folder dialogs
- Delete confirmation
