---
title: platform_management capability retired in favor of platform
description: "Capabilities gain a deprecated and retired lifecycle, and platform_management is the first one retired: it stays registered but no longer contributes tools."
date: 2026-09-11
kind: breaking
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, api]
release: "0.25.0"
links:
  docs: [/capabilities/platform-management/, /capabilities/platform/]
  prs: [everruns/everruns#3513]
---

Built-in capabilities now have a two-step removal lifecycle. A `deprecated`
capability keeps working while catalogs warn. A `retired` one stays registered so
agents that reference it still start, but it contributes no tools, prompt, mounts
or hooks, and is hidden from catalogs unless `include_retired` is set.

`platform_management` is retired. Its 14 hand-written tools are replaced by the
`platform` capability, which derives the same surface from the command catalog.
Agents that still list `platform_management` should switch to `platform`.

See [Platform Management (removed)](/capabilities/platform-management/) and
[Platform](/capabilities/platform/). [#3513](https://github.com/everruns/everruns/pull/3513)
