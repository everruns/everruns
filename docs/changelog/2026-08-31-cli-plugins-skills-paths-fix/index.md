---
title: Everruns CLI plugins, skills and knowledge-bases commands work again
description: The everruns plugins, skills and knowledge-bases commands called unversioned API paths and got 404 responses; they now use the /v1/ routes.
date: 2026-08-31
updated: 2026-10-23
kind: fix
significance: minor
products: [cli]
surfaces: [cli]
release: "0.24.0"
links:
  docs: [/reference/cli/plugins/, /reference/cli/skills/, /reference/cli/knowledge/]
  prs: [everruns/everruns#3326]
---

The `everruns plugins`, `everruns skills` and `everruns knowledge-bases` commands sent requests to `/plugins`, `/skills` and `/knowledge-bases`, which the server does not serve, so they failed with 404. They now call `/v1/plugins`, `/v1/skills` and `/v1/knowledge-bases` for both list and per-item requests.

Knowledge now uses the unified `everruns knowledge` command group; the undeployed `knowledge-bases` group has been retired. See [CLI: plugins](/reference/cli/plugins/), [CLI: skills](/reference/cli/skills/), [CLI: Knowledge](/reference/cli/knowledge/), the [Knowledge guide](/capabilities/knowledge/) and [#3326](https://github.com/everruns/everruns/pull/3326).
