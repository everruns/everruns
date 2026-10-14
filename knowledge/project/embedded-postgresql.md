---
type: Specification
title: "Embedded PostgreSQL"
description: "Why disposable dev databases use the PostgreSQL lifecycle library, and which responsibilities remain local."
tags:
  - everruns
  - project
  - testing
  - database
---
# Embedded PostgreSQL

## Decision

Dev mode and isolated server tests run a native PostgreSQL server through
[`postgresql_embedded`](https://docs.rs/postgresql_embedded/0.21.0/postgresql_embedded/).
It needs no Docker daemon. The same repositories and migrations run as in production;
there is no separate in-memory implementation of application storage.

The library owns downloading and verifying binaries, version selection, installation,
initialization, readiness, and normal shutdown. The private
[`everruns-pg-embedded` adapter](../../crates/pg-embedded/src/lib.rs) owns only the
process-wide instance, local security/performance settings, owner-exit cleanup, and
creating or dropping template copies. Server-specific schema and fixture ownership
stays in [`test_database.rs`](../../crates/server/src/storage/test_database.rs).

## Why keep a cleanup adapter

The library stops the server when its handle is dropped. A process-global shared
handle is never dropped when a test executable exits, and abrupt owner termination
also bypasses destructors. A standalone library probe confirmed that a normally
dropped handle removes the cluster, while `process::exit` leaves PostgreSQL running.

A pipe watcher covers this missing lifecycle boundary: owner termination closes the
pipe and the helper requests bounded PostgreSQL shutdown before removing its private
directory. It does not poll process IDs, download binaries, implement readiness, or
reimplement PostgreSQL startup. See the
[owner-exit regression tests](../../crates/pg-embedded/src/tests.rs).

## Execution contract

Embedded callers run as non-root users, including the standard GitHub Actions runners.
The library has no subprocess-credential hook. Maintaining a second initialization
and startup path for root would duplicate the lifecycle this change delegates.
Privileged environments use a non-root account or the existing external PostgreSQL
mode. Startup errors explain this choice.

Disposable clusters disable durability for functional tests. They prove actual SQL,
constraints, transactions, and locking, but do not establish crash persistence.
Persistence and recovery validation needs durable PostgreSQL settings.

The library version is pinned to its published compatibility line, with default
features disabled so it retains the existing archive source and Rustls backend.
The existing PostgreSQL binary cache is reused. Neither binaries nor archives are
bundled into shipped executables.

## Success bar

- Real queries, migrated template copies, and forced database drops still work.
- Normal owner exit and forced termination stop the server and remove private data.
- Cleanup leaves unrelated files and active PostgreSQL instances alone.
- A warm start reuses installed binaries without contacting a release service.
- Embedded and externally managed test databases use the same application SQL.
