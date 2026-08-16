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

Session-scoped SQLite databases for structured data storage. Create tables, insert data, and run queries, all isolated to the current session.

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

Introspect the database schema, list tables, columns, and types.

## Notes

- Database is session-scoped, destroyed when the session ends
- SELECT queries return at most 1000 rows
- Standard SQLite SQL syntax

## See Also

- [Storage](/capabilities/session-storage/), simpler key/value alternative
- [File System](/capabilities/file-system/), file-based data storage
- [Capabilities Overview](/capabilities/)
