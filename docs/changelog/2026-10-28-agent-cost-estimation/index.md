---
title: "Estimate agent model costs before running tasks"
description: "Plan per-task and monthly model spend from an agent's configuration, with editable workload scenarios, context growth and transparent pricing assumptions."
date: 2026-10-28
supersededBy: 2026-10-28-smart-agent-cost-projections
kind: feature
significance: notable
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui]
links:
  docs: [/features/ui/]
---

![Agent cost estimator showing a task estimate, monthly projection and editable workload assumptions](./estimator@2x.png)

The [Agent page](/features/ui/#estimate-agent-cost) now has **Estimate cost** in
the page header. Choose the kind of task and enter tasks per day to see the estimated
cost per task and per month. Optional **Advanced assumptions** contains token controls
and the detailed breakdown.

Estimates use the current draft's assembled instructions and tools, inherited model
selection and catalog prices. They account for growing conversation context,
long-context pricing and output limits. Missing prices remain unavailable.

The lighter-to-heavier range is a planning scenario, not a quote or spending limit.
Sandbox, paid-tool and platform fees are excluded. Opening the estimator does not
run or save the agent.
