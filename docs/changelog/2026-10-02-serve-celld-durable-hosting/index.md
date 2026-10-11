---
title: Run an Everruns serve app durably on celld
description: The everruns-serve-celld hosting target snapshots and restores a serve app's state, so sessions survive losing the serve process or the celld node mid-turn.
date: 2026-10-02
kind: feature
significance: notable
products: [framework]
surfaces: [code, operations]
release: "0.34.2"
links:
  docs: [/framework/serve-celld/]
  prs: [everruns/everruns#4019]
---

The new `everruns-serve-celld` hosting target runs a serve app on celld. The
serve binary exposes `/celld/state`, `/celld/snapshot` and `/celld/restore`,
and boots lazily so a restore always comes first.

The `examples/serve/celld` example adds the app, a Dockerfile and the celld
worker: a Durable Object per name that journals mutating requests, snapshots
the data directory when idle, and restores and replays when its container is
fresh. An end-to-end test kills serve mid-turn, and then the node itself.

See [Serve on celld](/framework/serve-celld/). PR [#4019](https://github.com/everruns/everruns/pull/4019).
