---
title: "api_endpoint channels convert to Agent API channels"
description: "Existing api_endpoint channels become Agent API channels at startup with the same keys, and old request shapes now return api_endpoint_retired or HTTP 410."
date: 2026-10-10
kind: breaking
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/how-to/call-your-agent-from-code/, /features/channels/#agent-api]
  prs: [everruns/everruns#4527]
---

The `api_endpoint` channel type is removed. At startup, each existing
`api_endpoint` channel converts to an Agent API (`api`) channel: its keys are
imported with the same hash, its sessions are retagged, and its settings map to
the new channel.

Clients must move to the Agent API routes under `/v1/channels/{channel_id}`.
Old request shapes get `400 api_endpoint_retired` pointing at the Agent API
docs, and `/v1/apps/{app}/api/{chan}` routes return `410`.

See [Call your agent from code](/how-to/call-your-agent-from-code/),
[Agent API](/features/channels/#agent-api) and [#4527](https://github.com/everruns/everruns/pull/4527).
