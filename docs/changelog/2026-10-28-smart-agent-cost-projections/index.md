---
title: "Plan agent costs with AI task suggestions and projections"
description: "Estimate typical tasks from your agent, adjust the task mix and expected users, and explore daily and monthly AI costs with live projections."
date: 2026-10-28
kind: improvement
significance: notable
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui, api]
links:
  docs: [/features/ui/]
---

![Agent cost dashboard with suggested tasks and usage projections](./projections@2x.png)

**Estimate cost** suggests typical tasks from your agent’s instructions and tools. Adjust the
task mix, then choose tasks per day or active users to explore daily and monthly costs.
The utility model proposes workloads; catalog prices determine model costs.

**Other operating costs** adds sandbox runtime, paid tools, media, subagent work and monthly
fees using your quantities and provider prices. Missing prices are flagged. **Advanced
assumptions** includes cache usage and a model retry allowance. Monthly fees and stored capacity count once,
even at zero task volume.

![Example sandbox runtime and provider-price inputs](./operating-costs@2x.png)

These are planning scenarios for included resources, not measured usage or spending limits.
The manual calculator remains available when smart estimates are unavailable. No tasks run
and the agent is not changed.

Open an agent and select [Estimate cost](/features/ui/#estimate-agent-cost).
