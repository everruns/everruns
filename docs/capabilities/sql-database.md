---
title: SQL Database
description: "Session-scoped SQLite databases: create tables, run queries, and persist relational data per session."
appliesTo: [platform, cloud]
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
| `database` | string | yes | Database name (alphanumeric and underscores); created if missing |
| `sql` | string | yes | SQL statement to execute |

### `sql_query`

Run SELECT queries. Results limited to 1000 rows.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `database` | string | yes | Database name |
| `sql` | string | yes | SELECT query |

### `sql_schema`

Introspect the database schema, list tables, columns, and types.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `database` | string | yes | Database name |
| `table` | string | no | Table name; omit to list all tables |

## Notes

- Database is session-scoped, destroyed when the session ends
- SELECT queries return at most 1000 rows
- Standard SQLite SQL syntax

## See Also

- [Storage](/capabilities/session-storage/), simpler key/value alternative
- [File System](/capabilities/file-system/), file-based data storage
- [Capabilities Overview](/capabilities/)
