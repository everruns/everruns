---
title: Sessions show where their commands run and what they may touch
description: Each session now declares its execution environment, readable through the API and shown in an Environment panel on the session Workspace tab.
date: 2026-09-14
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.27.0"
links:
  docs: [/framework/workspaces-and-environments/]
  prs: [everruns/everruns#3599]
visual: "Session Workspace tab showing the Environment panel (/sessions/<id>, Workspace tab); crop to the panel"
---

A session now has an explicit environment: where its commands run (compute) and
what they may touch (containment).

**In the UI:** the session **Workspace** tab gains an **Environment** panel.

**In the API:** `GET /v1/sessions/{id}/environment` and
`GET /v1/environment-targets` are read-only views of the same data. The feature is
enabled wherever sandboxes are.

**In code:** the Framework's `Environment` gains named compute and containment
members, with `HostCompute` behind the `host-compute` feature.

See [Workspaces and environments](/framework/workspaces-and-environments/). [#3599](https://github.com/everruns/everruns/pull/3599)
