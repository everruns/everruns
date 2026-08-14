# everruns-session-sqldb

> Session-scoped SQL databases for Everruns agents.

`everruns-session-sqldb` gives each Everruns session its own SQLite-compatible
database, backed by PostgreSQL page-level storage so the data is durable and
multi-tenant-isolated. It powers the SQL database capability that lets agents
create tables, run queries, and persist structured data across a session. An
in-memory backend is provided for local development and tests.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## Quick Example

```rust
use everruns_provider::typed_id::SessionId;
use everruns_session_sqldb::InMemorySqlDbBackend;

let backend = InMemorySqlDbBackend::new();
let session = SessionId::new();

// `execute` auto-creates the database on first use.
backend.execute(session, "notes", "CREATE TABLE notes (id INTEGER, body TEXT)").unwrap();
backend.execute(session, "notes", "INSERT INTO notes VALUES (1, 'hello')").unwrap();

let result = backend.query(session, "notes", "SELECT body FROM notes").unwrap();
assert_eq!(result.row_count, 1);
```

## What It Provides

- Per-session SQL databases isolated by session
- PostgreSQL page-level storage backend for durability
- An in-memory backend (`InMemorySqlDbBackend`) for development and tests
- Schema and result types (`TableSchema`, `SqlQueryResult`, `SqlExecuteResult`, …)

## Documentation

- [SQL database capability](https://docs.everruns.com/capabilities/sql-database/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
