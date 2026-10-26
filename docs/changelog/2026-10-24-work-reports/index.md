---
title: Work reports show what an agent is doing and got done
description: Each turn gets one report of its plan, steps, what it waits on and its result, shown in chat and the sessions list and returned by the API.
date: 2026-10-24
kind: feature
significance: major
products: [platform, cloud]
release: "0.49.0"
surfaces: [ui, api]
links:
  docs: [/features/work-reports/]
  prs: [everruns/everruns#4885, everruns/everruns#4890]
---

Every turn now has a work report: what the agent is doing right now and what it
got done. Chat pins the plan the agent shared ("Step 2 of 4") while it works and
ends with a result box, such as "Done · 2 files changed" with the agent's
summary and links. The sessions list and run cards show the headline of a
session still working. Finished shell steps say what came out ("Ran `cargo
test`, 3 of 12 tests failed"), and agents can call `report_done` to write their
own summary.

```bash
everruns sessions work get session_01h9
```

The same report is at `GET /v1/sessions/{id}/work` and, in a public view, in an
Agent API session read. See [Work reports](/features/work-reports/).
