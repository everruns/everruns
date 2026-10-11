---
title: Everruns CLI plugins, skills and knowledge-bases commands work again
description: The everruns plugins, skills and knowledge-bases commands called unversioned API paths and got 404 responses; they now use the /v1/ routes.
date: 2026-08-31
kind: fix
significance: minor
products: [cli]
surfaces: [cli]
release: "0.24.0"
links:
  docs: [/reference/cli/plugins/, /reference/cli/skills/, /reference/cli/knowledge-bases/]
  prs: [everruns/everruns#3326]
---

The `everruns plugins`, `everruns skills` and `everruns knowledge-bases` commands sent requests to `/plugins`, `/skills` and `/knowledge-bases`, which the server does not serve, so they failed with 404. They now call `/v1/plugins`, `/v1/skills` and `/v1/knowledge-bases` for both list and per-item requests.

See [CLI: plugins](/reference/cli/plugins/), [CLI: skills](/reference/cli/skills/), [CLI: knowledge-bases](/reference/cli/knowledge-bases/) and [#3326](https://github.com/everruns/everruns/pull/3326).
