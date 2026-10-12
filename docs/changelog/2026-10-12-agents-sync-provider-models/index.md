---
title: "Agents can sync provider models with the Platform capability"
description: "sync_provider_models now works from an agent, and provider create and update from an agent discover models and set the organization's default model."
date: 2026-10-12
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/platform/]
  prs: [everruns/everruns#4405]
---

An agent with the [Platform capability](/capabilities/platform/) can now run
`sync_provider_models`. Before, the command answered "Model sync is not
available on this endpoint" when an agent ran it, although it worked over the
REST API and MCP.

Creating or updating a provider from an agent now also discovers the
provider's models and sets the organization's default model when none is set,
the same as the Providers API.

See [#4405](https://github.com/everruns/everruns/pull/4405).
