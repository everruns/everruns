---
title: "Plan agent costs with AI task suggestions and projections"
description: "Estimate typical tasks from your agent, adjust the task mix and expected users, and explore daily and monthly AI costs with live projections."
date: 2026-10-28
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/]
---

![Agent cost dashboard with suggested tasks and usage projections](./projections@2x.png)

**Estimate cost** now suggests typical tasks from your agent’s instructions and tools.
Adjust the task mix, then move the **Tasks per day** slider or switch to **By users**
to plan active users and tasks per user.

The dashboard updates average task, daily and monthly AI costs immediately, with a chart
showing spend as usage grows. Optional workload guidance generates new suggestions;
**Advanced assumptions** lets you edit them directly. The utility model proposes workloads,
while current catalog prices determine costs.

These are planning scenarios, not measured usage or spending limits. Paid tools and hosting
are extra. The manual calculator remains available when smart estimates are unavailable.
No tasks are executed and the agent is not changed.

Open an agent and select [Estimate cost](/features/ui/#estimate-agent-cost).
