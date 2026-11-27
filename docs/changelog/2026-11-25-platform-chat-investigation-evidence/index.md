---
title: Investigate background work from Platform Chat
description: Inspect original alerts, attributable trigger history, follow-up schedules and filtered cost breakdowns through existing platform tools and CLI commands.
date: 2026-11-25
kind: improvement
significance: notable
products: [platform]
surfaces: [agent, api, cli, ui]
links:
  docs: [/features/agent-triggers/]
---

Platform Chat can inspect a session's follow-up schedules and investigate
[agent trigger](/features/agent-triggers/) runs through the existing platform
commands. Trigger history supports time windows and pagination, and records the
authenticated manual requester and resulting input message when available.

Health issues preserve the evidence that opened each new episode. Rechecking an
issue updates its current evidence while retaining the original observation.
Cost summaries support combined agent, session, source and starter filters with
joint breakdowns, and session costs accept explicit time windows. Reads also work
for sessions running on the data plane. Historical evidence that was never
recorded remains unknown; platform cost estimates do not establish an external
provider bill.

For example, compare sources and starters within each agent session:

```bash
everruns costs summary get --group-by agent,session,source,started_by
```

Costs keep unrecorded starters separate from people:

![Costs show Unknown when the starter was not recorded](./unknown-starter@2x.png)
