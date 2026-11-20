---
title: Eval import reads Harbor's draft ATIF v2 trajectories
description: Trajectories in Harbor's draft ATIF v2.0 format, including bundles with subagents, can seed eval cases alongside ATIF v1 documents.
date: 2026-11-20
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [api, cli]
links:
  docs: [/features/evals/]
---

The ATIF import that turns recorded trajectories into eval cases now also reads
Harbor's draft ATIF v2.0 format, so harnesses that move to v2 can keep seeding
datasets. A v2 bundle imports its root trajectory as one case, not one case per
subagent or rewind branch. See [Evals](/features/evals/#harbor-trajectories).

An event stream (`.atif-e.jsonl`) is refused with a clear error: consolidate it
into a trajectory first. Exports stay on ATIF v1.7, which Harbor upgrades to v2
without loss.
