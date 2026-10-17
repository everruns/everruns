---
title: Deno Deploy sandbox integration removed
description: The experimental Deno Deploy sandbox provider, its deno_* tools, the deno-coder demo agent, and related docs are removed.
date: 2026-10-16
cloud: 2026-10-17
kind: breaking
significance: minor
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/capabilities/]
---

The experimental Deno Deploy sandbox provider is removed. The `deno_*`
sandbox tools, the Deno connector and feature flag, leased-resource cleanup
for Deno sandboxes, and the `deno-coder` demo agent are gone, along with the
Deno rows in the capability catalog and docs.

Existing Deno connections and sessions that reference
`deno_*` tools will no longer resolve; use Daytona, E2B, Sprites, Modal, or
the container sandbox instead.

See the [capabilities overview](/capabilities/) for the supported sandbox
providers.

```bash
# Deno connections no longer resolve; use a supported sandbox provider
everruns connections set daytona
everruns connections list
```
