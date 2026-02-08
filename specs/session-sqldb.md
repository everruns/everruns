# Session SQL Database Specification

## Abstract

Session-scoped SQLite databases backed by PostgreSQL page-level storage. Each session can have multiple named SQLite databases. In production, a custom SQLite VFS stores 4KB pages as rows in PostgreSQL. In DEV_MODE, databases live in-memory using rusqlite's serialize/deserialize. Agents interact via standard SQL through capability tools.

## Design Decisions

### Decision 1: Page-Level VFS over Blob Storage
**Chosen:** Custom SQLite VFS stores individual 4KB pages in PostgreSQL rows
**Alternatives considered:**
- Full blob (serialize entire DB as BYTEA): Simple but 50MB blobs per query wasteful
- Table-level blobs: Doesn't map to SQLite's B-tree page architecture
- PostgreSQL Large Objects: Quirky API, less control than explicit rows
**Rationale:** Only dirty pages written per mutation (typically 3-5 pages regardless of DB size). SQLite's internal page cache (default ~8MB) serves hot reads from memory. No size ceiling — 50MB is just ~12K rows.

### Decision 2: sqlite-plugin Crate for VFS
**Chosen:** `sqlite-plugin` crate (v0.5.0) for VFS registration
**Alternatives considered:**
- `sqlite-vfs` crate: Unmaintained since 2022, prototype quality
- Raw FFI via `libsqlite3-sys`: Maximum control but maximum unsafety
**Rationale:** Centralized `Vfs` trait model fits PostgreSQL connection pooling. Falls back to raw FFI if integration issues arise.

### Decision 3: Live Connections for DEV_MODE
**Chosen:** Persistent `Arc<Mutex<Connection>>` per database in-memory
**Alternatives considered:**
- Serialize/deserialize per access: rusqlite 0.32 `serialize`/`deserialize` API is cumbersome (`OwnedData`, `DatabaseName`) and unnecessary overhead
- Full VFS for DEV_MODE too: Unnecessary complexity
- No DEV_MODE support: Breaks dev workflow
**Rationale:** Same API surface via trait. Each database holds a live in-memory SQLite connection. No VFS complexity for development. Size estimated via `conn.serialize()` length.

### Decision 4: Rollback Journal Mode
**Chosen:** `PRAGMA journal_mode=MEMORY` (rollback journal in memory)
**Alternatives considered:**
- WAL mode: Requires shared memory VFS methods (xShmMap/xShmLock), significantly more complex
- DELETE journal mode: Creates journal files that VFS must handle
**Rationale:** Memory journal avoids journal file handling entirely. Acceptable for session-scoped databases that don't need crash recovery beyond PostgreSQL's own durability.

### Decision 5: Advisory Locks for Concurrency
**Chosen:** PostgreSQL advisory locks per database for write serialization
**Alternatives considered:**
- Row-level locks on pages: Too granular, risk of deadlocks
- Application-level mutex: Doesn't work across server instances
**Rationale:** `pg_advisory_xact_lock(hashtext(database_id))` serializes writers per database. Reads are concurrent. Lock auto-released on transaction commit.

### Decision 6: Own Crate
**Chosen:** `crates/session-sqldb` isolates all SQLite/VFS logic
**Alternatives considered:**
- Inline in server crate: Mixes concerns
- In core crate: Core should stay lightweight
**Rationale:** Clean separation. Crate owns rusqlite dependency, VFS implementation, query execution, and security (authorizer). Server and core depend on it for types and backends.

### Decision 7: Auto-Create on Execute
**Chosen:** `sql_execute` tool auto-creates database if it doesn't exist
**Alternatives considered:**
- Require explicit create: Extra tool call overhead for LLMs
- Auto-create on all tools: `sql_query` on nonexistent DB should error
**Rationale:** Reduces friction. Agent can `sql_execute("main", "CREATE TABLE ...")` without a separate create step.

## Data Model

### SessionDatabase

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session (FK, CASCADE) |
| `name` | string | Database name (validated) |
| `size_bytes` | i64 | Total size of all pages |
| `page_count` | i32 | Number of stored pages |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### SessionDatabasePage

| Field | Type | Description |
|-------|------|-------------|
| `database_id` | UUID v7 | Parent database (FK, CASCADE) |
| `page_number` | i32 | Page index (0-based) |
| `data` | bytes | Page content (4096 bytes) |

Primary key: `(database_id, page_number)`

## Functional Requirements

### Database Management
- Create named databases within a session
- List all databases for a session
- Get metadata for a specific database
- Delete a database and all its pages
- Cascade delete when session is deleted (FK CASCADE)

### Query Execution
- Execute read-only SQL (SELECT) returning columns and rows as JSON
- Execute write SQL (CREATE TABLE, INSERT, UPDATE, DELETE) returning affected row count
- Auto-create database on first write operation
- Standard SQLite SQL syntax support

### Schema Introspection
- List all tables in a database
- Get column info (name, type, nullable, primary key, default) per table
- Get approximate row count per table

### Validation & Limits
- Database names: `^[a-zA-Z_][a-zA-Z0-9_]{0,63}$`
- Max 10 databases per session
- Max 50 MB per database
- Max 100 MB total per session
- Max 1000 result rows per query
- Max 1 MB result payload per query
- Query timeout: 30 seconds

### Row Serialization
| SQLite Type | JSON Type |
|-------------|-----------|
| INTEGER | number |
| REAL | number |
| TEXT | string |
| BLOB | string (base64) |
| NULL | null |

## API Endpoints

All under `/v1/sessions/{session_id}/databases`.

### List Databases

```http
GET /v1/sessions/{session_id}/databases

Response 200:
{
  "data": [
    {
      "name": "analytics",
      "size_bytes": 12288,
      "page_count": 3,
      "created_at": "2024-01-01T00:00:00Z",
      "updated_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```

### Create Database

```http
POST /v1/sessions/{session_id}/databases
Content-Type: application/json

{ "name": "analytics" }

Response 201:
{
  "name": "analytics",
  "size_bytes": 0,
  "page_count": 0,
  "created_at": "...",
  "updated_at": "..."
}
```

### Get Database

```http
GET /v1/sessions/{session_id}/databases/{name}

Response 200:
{
  "name": "analytics",
  "size_bytes": 12288,
  "page_count": 3,
  "created_at": "...",
  "updated_at": "..."
}
```

### Delete Database

```http
DELETE /v1/sessions/{session_id}/databases/{name}

Response 204 No Content
```

### Get Schema

```http
GET /v1/sessions/{session_id}/databases/{name}/schema

Response 200:
{
  "database": "analytics",
  "tables": [
    {
      "name": "users",
      "columns": [
        { "name": "id", "type": "INTEGER", "notnull": true, "pk": true, "default_value": null },
        { "name": "email", "type": "TEXT", "notnull": true, "pk": false, "default_value": null }
      ],
      "row_count": 42
    }
  ]
}
```

## Capability & Tools

### Capability

- **ID**: `session_sql_database`
- **Name**: SQL Database
- **Status**: Available
- **Icon**: `database`
- **Category**: Data
- **Dependencies**: None

### Tools

#### sql_execute

Execute DDL/DML SQL. Auto-creates database if it doesn't exist.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `database` | string | yes | Database name |
| `sql` | string | yes | SQL statement(s) |

**Response:**
```json
{ "database": "main", "success": true, "rows_affected": 1 }
```

#### sql_query

Execute read-only SQL query. Returns columns and rows.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `database` | string | yes | Database name |
| `sql` | string | yes | SELECT query |

**Response:**
```json
{
  "database": "main",
  "columns": ["id", "name", "age"],
  "rows": [[1, "Alice", 30], [2, "Bob", 28]],
  "row_count": 2
}
```

#### sql_schema

Introspect database schema. Optionally filter to a single table.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `database` | string | yes | Database name |
| `table` | string | no | Specific table name |

**Response:**
```json
{
  "database": "main",
  "tables": [
    {
      "name": "users",
      "columns": [
        { "name": "id", "type": "INTEGER", "notnull": true, "pk": true, "default_value": null }
      ],
      "row_count": 42
    }
  ]
}
```

### System Prompt Addition

```
You have access to session-scoped SQL databases. Use these tools to create tables, insert data, and query data using standard SQLite SQL syntax.

**Tools:**
- `sql_execute`: Create tables, insert/update/delete data. Auto-creates the database if needed.
- `sql_query`: Run SELECT queries. Returns columns and rows as JSON.
- `sql_schema`: Inspect database schema (tables, columns, types).

**Guidelines:**
- Database names must be alphanumeric with underscores (e.g., "analytics", "user_data")
- Use `sql_schema` to inspect existing tables before querying
- Results are limited to 1000 rows per query
- Databases are session-scoped and isolated
- Standard SQLite SQL syntax is supported
```

## Threat Model

### T1: Dangerous SQLite Operations
**Risk:** Agent uses ATTACH to access other databases, LOAD_EXTENSION to run arbitrary code, or modifies SQLite configuration via PRAGMAs.
**Mitigation:** SQLite authorizer callback blocks:
- ATTACH / DETACH
- load_extension function
- CREATE/DROP VIRTUAL TABLE
- Write-mode PRAGMAs (journal_mode, page_size, locking_mode, synchronous, wal_*, mmap_size)
**Allowed PRAGMAs:** table_info, table_list, table_xinfo, index_list, index_info, database_list, foreign_key_list (read-only introspection)

### T2: Storage Exhaustion
**Risk:** Agent creates huge databases consuming unbounded storage.
**Mitigation:** Hard limits enforced at VFS write and create levels:
- 50 MB per database
- 100 MB total per session
- 10 databases per session
- VFS returns SQLITE_FULL when size exceeded

### T3: CPU Exhaustion
**Risk:** Expensive queries (cartesian joins, infinite recursive CTEs).
**Mitigation:**
- SQLite progress handler interrupts after 30 seconds
- tokio::time::timeout wrapping blocking query execution

### T4: Filesystem Escape
**Risk:** SQLite tries to access files outside VFS.
**Mitigation:**
- Custom VFS intercepts ALL file I/O — no real filesystem access
- Authorizer blocks ATTACH (prevents opening other files)
- DEV_MODE uses Connection::open_in_memory() — no disk access

### T5: Cross-Session Data Access
**Risk:** Session A reads Session B's databases.
**Mitigation:**
- VFS file names use database UUID (not user-controlled names)
- All PG queries filter through database_id → session_id FK chain
- Authorizer blocks ATTACH (can't reference foreign UUIDs)

### T6: Concurrent Write Corruption
**Risk:** Parallel tool calls write to same database simultaneously.
**Mitigation:**
- PostgreSQL advisory lock per database for write serialization
- DEV_MODE uses RwLock per database entry
- Reads are concurrent, writes are serialized

### T7: Result Size Exhaustion
**Risk:** SELECT returns millions of rows exhausting server memory.
**Mitigation:**
- Max 1000 result rows
- Max 1 MB result payload
- Executor stops reading after limit, appends truncation notice

## DEV_MODE

In-memory backend provides full API parity without PostgreSQL:

1. `Connection::open_in_memory()` for each database, held as `Arc<Mutex<Connection>>`
2. Live connections persist for session lifetime — no serialize/deserialize round-trips
3. Size tracking via `conn.serialize(DatabaseName::Main)` length estimation
4. No VFS complexity — standard rusqlite in-memory mode
5. Concurrency via `RwLock` on the database map, `Mutex` per connection
6. Async wrapper (`InMemorySqlDbStore`) uses `tokio::task::spawn_blocking`

## Future Extensibility

The internal `SqlDbBackend` trait can support different scopes:

```rust
enum DatabaseScope {
    Session(SessionId),
    // Future:
    // Organization(OrgId),
    // Agent(AgentId),
}
```

Org-level databases would share the same crate, VFS, and query execution — only the API routing and FK relationships change.

## Database Schema

```sql
CREATE TABLE session_databases (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT session_databases_unique_name UNIQUE (session_id, name),
    CONSTRAINT session_databases_name_check CHECK (name ~ '^[a-zA-Z_][a-zA-Z0-9_]{0,63}$')
);

CREATE INDEX idx_session_databases_session_id ON session_databases(session_id);

CREATE TRIGGER update_session_databases_updated_at
    BEFORE UPDATE ON session_databases
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE session_database_pages (
    database_id UUID NOT NULL REFERENCES session_databases(id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL,
    data BYTEA NOT NULL,
    PRIMARY KEY (database_id, page_number)
);

CREATE INDEX idx_session_database_pages_db ON session_database_pages(database_id);
```

## Implementation Status

- [x] Core types and async trait (`crates/core/src/session_sqldb.rs`)
- [x] Capability + 3 tools (`crates/core/src/capabilities/session_sql_database.rs`)
- [x] In-memory backend (`crates/session-sqldb/src/memory.rs`)
- [x] Async store wrapper (`crates/session-sqldb/src/store.rs`)
- [x] Query executor with authorizer and limits (`crates/session-sqldb/src/executor.rs`)
- [x] HTTP API routes (`crates/server/src/api/session_databases.rs`)
- [x] Wired into server main, worker adapters, ActAtom
- [x] Seed agent: Data Analyst
- [x] Integration tests (CRUD, schema, validation)
- [x] E2E verified: agent session with LLM using sql_execute/sql_query tools
- [ ] PostgreSQL VFS backend (production)
- [ ] UI integration
- [ ] Load testing (concurrent queries, size limit enforcement, connection pool exhaustion)

## UI Integration (Future)

- "Databases" tab on session detail page
- Database list with name, size, page count
- Schema viewer per database (tables, columns)
- SQL query runner with results table
- Create/delete database dialogs
