---
title: "Session SQL databases survive server restarts"
description: "Session SQL databases are now stored in PostgreSQL page by page, so a server restart no longer loses them and every server instance sees the same data."
date: 2026-10-10
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/sql-database/]
  prs: [everruns/everruns#4553]
---

Databases created with the SQL database capability lived only in one server
process's memory, so a restart lost them and other server instances never saw
them.

The server now keeps an in-memory SQLite cache per database and, after every
write, stores only the changed 4 KB pages in PostgreSQL in one transaction. A
server that does not have the database cached rebuilds it from the stored
pages, and each commit is a compare-and-set so concurrent writers cannot
overwrite each other. Agents keep using `sql_execute`, `sql_query` and
`sql_schema` as before.

See [SQL database](/capabilities/sql-database/) and [#4553](https://github.com/everruns/everruns/pull/4553).
