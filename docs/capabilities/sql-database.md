---
title: SQL Database
description: Session-scoped SQLite databases for structured data storage and querying. Agents can create tables, run SQL queries, and persist relational data per session.
---

| | |
|---|---|
| **ID** | `session_sql_database` |
| **Category** | Data |
| **Features** | `sql_database` |
| **Dependencies** | None |

Session-scoped SQLite databases for structured data storage. Create tables, insert data, and run queries — all isolated to the current session.

## Tools

### `sql_execute`

Run DDL/DML statements (CREATE TABLE, INSERT, UPDATE, DELETE).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sql` | string | yes | SQL statement to execute |

### `sql_query`

Run SELECT queries. Results limited to 1000 rows.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sql` | string | yes | SELECT query |

### `sql_schema`

Introspect the database schema — list tables, columns, and types.

## Use Cases

- **Data analysis** — import CSV/JSON data into tables and run analytical queries
- **Structured storage** — store relational data that doesn't fit key/value patterns
- **Reporting** — aggregate and summarize data with SQL
- **Prototyping** — test database schemas and queries before implementing

## Example

```
User: Track my reading list

Agent:
  → sql_execute("CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT, author TEXT, status TEXT DEFAULT 'unread')")
  → sql_execute("INSERT INTO books (title, author) VALUES ('Designing Data-Intensive Applications', 'Martin Kleppmann')")
  → sql_execute("INSERT INTO books (title, author, status) VALUES ('The Pragmatic Programmer', 'Hunt & Thomas', 'read')")
  → sql_query("SELECT title, author, status FROM books ORDER BY title")
  ← [{ "title": "Designing Data-Intensive Applications", "author": "Martin Kleppmann", "status": "unread" }, ...]
```

## Notes

- Database is session-scoped — destroyed when the session ends
- SELECT queries return at most 1000 rows
- Standard SQLite SQL syntax

## See Also

- [Storage](/capabilities/session-storage/) — simpler key/value alternative
- [File System](/capabilities/file-system/) — file-based data storage
- [Capabilities Overview](/capabilities/)
