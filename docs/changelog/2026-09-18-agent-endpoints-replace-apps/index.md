---
title: Agent endpoints and triggers replace Apps in Everruns
description: "App management is retired: an agent's endpoints and triggers now live on its Integrations tab, with an org-wide Exposures view and per-agent suspend."
date: 2026-09-18
kind: breaking
significance: major
products: [platform, cloud]
surfaces: [ui, api]
release: "0.30.0"
links:
  docs: [/features/channels/, /features/agent-triggers/]
  prs: [everruns/everruns#3698, everruns/everruns#3624, everruns/everruns#3627, everruns/everruns#3621, everruns/everruns#3618, everruns/everruns#3607]
visual: "Agent Integrations tab with endpoint rows and the suspend switch (/agents/<id>, Integrations tab); crop to the endpoints section"
---

The App abstraction is retired. Endpoints (Slack, webhook, AG-UI, A2A, public chat
and others) and triggers now belong to the agent.

**In the UI:** an agent's **Integrations** tab lists its endpoints and triggers,
each with its own publish switch and real endpoint URL, plus a switch to suspend
all of the agent's exposures at once. A new **Exposures** page lists every endpoint
and trigger in the organization, flags anonymous live ones, and can suspend or
resume an agent. App management pages are gone.

**In the API:** publishing moves one endpoint. Suspend with
`POST /v1/agents/{agent_id}/exposures/suspend` and resume with `.../resume`. App
write and command surfaces are removed; existing App records are frozen and
ingress URLs keep working.

See [Channels](/features/channels/) and [Agent triggers](/features/agent-triggers/). [#3698](https://github.com/everruns/everruns/pull/3698), [#3624](https://github.com/everruns/everruns/pull/3624), [#3627](https://github.com/everruns/everruns/pull/3627), [#3621](https://github.com/everruns/everruns/pull/3621)
