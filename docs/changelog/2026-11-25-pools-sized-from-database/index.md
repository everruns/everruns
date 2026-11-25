---
title: Database pools size themselves from PostgreSQL's connection limit
description: Servers read max_connections at startup and split it across instances, so resizing PostgreSQL no longer needs a matching pool setting.
date: 2026-11-25
kind: feature
significance: minor
products: [platform]
surfaces: [ops]
links:
  docs: [/sre/environment-variables/]
---

Resizing PostgreSQL used to mean editing `DATABASE_POOL_MAX` by hand: too
high and servers ran out of connections at startup, too low and requests
queued behind each other. Servers now ask the database for its
`max_connections` when they start, hold back PostgreSQL's reserved slots and a
share for migrations, workers and operators, and split the rest across
`EXPECTED_INSTANCES`. The request pool never drops below three connections; a
database too small for that gets a startup warning.

Pools you size explicitly stay as you set them. With a transaction pooler such
as PgBouncer on `DATABASE_URL`, point `DATABASE_UNPOOLED_URL` at PostgreSQL
directly: listeners, session locks, migrations and background work now all use
it. `DATABASE_POOLER_ADDR` sends request traffic through such a pooler with
the same credentials. See [environment variables](/sre/environment-variables/).
