---
title: Repeating schedules now pause themselves, with one-click resume
description: When a schedule keeps ending the same way, Everruns pauses it at 24 runs, records it on the finding, and lets you resume it from there.
date: 2026-11-02
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5173]
---

A schedule that polls for something that never changes used to burn 48 runs
before the runaway stop kicked in. Now the [finding](/features/evals/) that
opens at run 24 does something about it: Everruns pauses the schedule, tells
the schedule's owner and the agent's authors, and shows what it did on the
finding page with a **Resume schedule** button. Resuming starts the
repeated-run count over.

![A finding for a schedule that repeated itself 24 times. The right column says "Done automatically: Paused the schedule" with a Resume schedule button](./finding-paused@2x.png)

Each organization picks how far this goes under **Settings → Limits → When a
finding opens**: pause it, offer to pause it (the finding gets a **Pause
schedule** button instead), or do nothing. The 48-run stop still applies
whatever you pick.

Changes to an agent are never automatic. The Inspector's
[suggested fixes](/changelog/2026-11-02-suggested-fixes/) still wait for
a person to check and approve them, and an organization can turn the
suggestions off.

The same record is in the CLI:

```bash
everruns health-issues actions list --issue-id 0193...
everruns health-issues actions undo --issue-id 0193... --action-id fact_0193... --reason 'Claim still pending'
```
