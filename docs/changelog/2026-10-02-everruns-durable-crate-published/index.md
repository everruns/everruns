---
title: everruns-durable workflow engine published on crates.io
description: The durable workflow engine behind Everruns ships as its own crate with timers, child workflows and a bundled idempotent PostgreSQL schema you can migrate.
date: 2026-10-02
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.34.1"
links:
  docs: [/explanation/durable-execution/]
  prs: [everruns/everruns#3994, everruns/everruns#4008]
---

`everruns-durable`, the generic durable workflow engine with no agent
concepts, is now published on crates.io.

It ships its own idempotent PostgreSQL schema through
`PostgresWorkflowEventStore::migrate` and `SCHEMA_SQL`, tested to match what
the server migrations build. Timers and child workflows run as engine tasks:
`ActivityOptions::start_delay` holds a task until it is due, and parents get
`on_child_workflow_completed` and `on_child_workflow_failed`.

```bash
cargo add everruns-durable
```

See [Durable execution](/explanation/durable-execution/). PRs
[#3994](https://github.com/everruns/everruns/pull/3994), [#4008](https://github.com/everruns/everruns/pull/4008).
