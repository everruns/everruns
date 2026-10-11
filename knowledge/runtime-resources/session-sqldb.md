---
type: Specification
title: "Session SQL Database Specification"
description: "Session-scoped SQL databases."
tags:
  - everruns
  - runtime-resources
---
# Session SQL Database Specification

## Abstract

Session-scoped SQLite databases persisted in PostgreSQL as 4 KB pages. Each session can have multiple named SQLite databases. Each server process keeps a database open in memory as a working cache; after every write it diffs the database's pages against what was last persisted and commits only the changed pages. A process that has no cache (first access, restart, another instance) rebuilds the database from the stored pages. Agents interact via standard SQL through capability tools.

The PostgreSQL store is `crates/server/src/session_sqldb/postgres/mod.rs` (page images in `pages.rs`, SQL in `crates/server/src/storage/repositories/session_databases.rs`). The in-memory backend (`memory.rs`, `store.rs`) remains for tests and embedded uses with no PostgreSQL.

## Design Decisions

### Decision 1: In-Memory Cache + Page Diff over a Custom VFS
**Chosen:** Keep each database as an in-memory SQLite connection; after a write, serialize it (`sqlite3_serialize`), compare each page's SHA-256 with the digest of the persisted page, and upsert only the changed pages (removing pages past the new end) in one PostgreSQL transaction.
**Alternatives considered:**
- Custom SQLite VFS writing pages straight to PostgreSQL: true dirty-page tracking, but a large unsafe FFI surface, a new crate, and blocking PostgreSQL I/O inside SQLite callbacks
- Full blob (serialize entire DB as BYTEA): simple, but rewrites up to 50 MB per write
- Table-level blobs: doesn't map to SQLite's B-tree page architecture
**Rationale:** Writes stay proportional to what changed (a one-row insert touches a handful of pages), with no new dependency: rusqlite's `serialize` feature was already enabled. The cost is one in-memory serialization and hash pass per write, bounded by the 50 MB database limit. The cache keeps page digests, not a second copy of the image.

### Decision 2: PostgreSQL Is the Source of Truth
**Chosen:** A write is durable before the tool call returns; the cache is disposable.
**Rationale:** Restarts and other server instances see every acknowledged write. Each `sql_execute` call is all-or-nothing: if it fails after changing the cache (a later statement in the batch errors, a size limit is hit, the commit fails), the cache is dropped and the next call reloads the committed state. A call may not leave a transaction open, because an open transaction's pages are not committed state. Calls that change nothing (reads, a SELECT through `sql_execute`) write nothing. The cache is bounded per process; evicting only costs a reload.

### Decision 3: Read-Only Queries Are Enforced
**Chosen:** `sql_query` and `sql_schema` refuse statements SQLite does not report as read-only, and refuse leaving a transaction open.
**Rationale:** Reads never persist pages, so a write slipped through a query would live in one process's cache and vanish on reload.

### Decision 4: Rollback Journal Mode
**Chosen:** `PRAGMA journal_mode=MEMORY` (rollback journal in memory)
**Alternatives considered:**
- WAL mode: Requires shared memory VFS methods (xShmMap/xShmLock), significantly more complex
- DELETE journal mode: Creates journal files outside the in-memory database
**Rationale:** Memory journal avoids journal file handling entirely. Acceptable for session-scoped databases that don't need crash recovery beyond PostgreSQL's own durability.

### Decision 5: Generation Compare-and-Set for Concurrency
**Chosen:** `session_databases.generation`, bumped by every page commit, which only applies when the stored generation still equals the one the writer's cache was loaded at.
**Alternatives considered:**
- Advisory lock or row lock held while the statement runs: holds a PostgreSQL connection for up to the 30-second query timeout
- Row-level locks on pages: too granular, risk of deadlocks
**Rationale:** Every access re-reads the generation and reloads a stale cache before using it, so a process never queries or writes on top of another process's outdated view. A writer that loses the race between its check and its commit writes nothing, reloads, and re-runs the statement (bounded retries). Within a process, one async mutex per database serializes access to its cache.

### Decision 6: Server-Owned Module
**Chosen:** `crates/server/src/session_sqldb` isolates all SQLite and page-storage logic inside its only deployment owner
**Alternatives considered:**
- Standalone leaf crate: Adds package and release overhead without another consumer
- In core crate: Core should stay lightweight
**Rationale:** A focused server module keeps the rusqlite dependency, page persistence, query execution, and authorizer isolated without presenting a false reusable boundary. Portable contracts remain in platform; core stays lightweight.

### Decision 7: Auto-Create on Execute
**Chosen:** `sql_execute` tool auto-creates database if it doesn't exist
**Alternatives considered:**
- Require explicit create: Extra tool call overhead for LLMs
- Auto-create on all tools: `sql_query` on nonexistent DB should error
**Rationale:** Reduces friction. Agent can `sql_execute("main", "CREATE TABLE ...")` without a separate create step.

## Data Model

See `crates/server/src/session_sqldb/types.rs` for the backend result types. See `crates/server/migrations/` for the database schema DDL.

## Functional Requirements

### Database Management
- Create named databases within a session
- List all databases for a session
- Get metadata for a specific database
- Delete a database and all its pages
- Cascade delete when session is deleted (FK CASCADE)
- Session fork copies every database page for page into the child (`Database::copy_session_databases`, one statement, so a consistent snapshot)

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
**Mitigation:** Hard limits enforced on create and before every page commit:
- 50 MB per database
- 100 MB total per session (summed from stored sizes)
- 10 databases per session
- A write that would exceed a limit fails and its changes are discarded, nothing is persisted

### T3: CPU Exhaustion
**Risk:** Expensive queries (cartesian joins, infinite recursive CTEs).
**Mitigation:**
- SQLite progress handler interrupts after 30 seconds
- tokio::time::timeout wrapping blocking query execution

### T4: Filesystem Escape
**Risk:** SQLite tries to access files on the host filesystem.
**Mitigation:**
- Databases are only ever opened in memory (`open_in_memory` plus `deserialize`), no disk access
- Authorizer blocks ATTACH (prevents opening other files)

### T5: Cross-Session Data Access
**Risk:** Session A reads Session B's databases.
**Mitigation:**
- Caches are keyed by (session id, name); stored pages are keyed by the database UUID, reached only through a session-scoped metadata lookup
- All PG queries filter through database_id → session_id FK chain
- Authorizer blocks ATTACH (can't reference foreign UUIDs)

### T6: Concurrent Write Corruption
**Risk:** Parallel tool calls write to same database simultaneously.
**Mitigation:**
- Generation compare-and-set on every page commit; a stale writer reloads instead of overwriting
- One async mutex per database serializes calls within a process

### T7: Result Size Exhaustion
**Risk:** SELECT returns millions of rows exhausting server memory.
**Mitigation:**
- Max 1000 result rows
- Max 1 MB result payload
- Executor stops reading after limit, appends truncation notice

## In-Memory Backend

`InMemorySqlDbBackend` (wrapped by `InMemorySqlDbStore`) keeps live in-memory connections with no persistence. It serves tests and embedded uses that have no PostgreSQL; the server always wires the PostgreSQL store.

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

Org-level databases would share the same module, page storage, and query execution, only the API routing and FK relationships change.

## Database Schema

The `session_databases` and `session_database_pages` tables, columns, constraints
(unique name, name-format check), indexes, and the `updated_at` trigger, are
defined in `crates/server/migrations/001_base_schema.sql`; the write generation
was added by `218_session_database_generation.sql`. `session_database_pages`
holds the paged SQLite backing store keyed by `(database_id, page_number)`,
numbered from 1 like SQLite's own pages.

## Implementation Status

- [x] Portable types and async trait (`crates/contracts/src/session_sqldb.rs`)
- [x] Capability + 3 tools (`crates/capabilities/src/capabilities/session_sql_database.rs`)
- [x] In-memory backend (`crates/server/src/session_sqldb/memory.rs`)
- [x] Async store wrapper (`crates/server/src/session_sqldb/store.rs`)
- [x] Query executor with authorizer and limits (`crates/server/src/session_sqldb/executor.rs`)
- [x] HTTP API routes (`crates/server/src/api/session_databases.rs`)
- [x] Wired into server main, worker adapters, ActAtom
- [x] Seed agent: Data Analyst
- [x] Integration tests (CRUD, schema, validation)
- [x] E2E verified: agent session with LLM using sql_execute/sql_query tools
- [x] gRPC support: proto RPCs, GrpcSessionSqlDbStore adapter, server handlers
- [x] WorkerAdapters::sqldb_store() returns non-optional (enforced at compile time)
- [x] PostgreSQL persistence via in-memory cache + page diff (`crates/server/src/session_sqldb/postgres/mod.rs`)
- [x] Session fork copies databases
- [ ] UI integration
- [ ] Load testing (concurrent queries, size limit enforcement, connection pool exhaustion)

## UI Integration (Future)

- "Databases" tab on session detail page
- Database list with name, size, page count
- Schema viewer per database (tables, columns)
- SQL query runner with results table
- Create/delete database dialogs
