---
title: Trigger and schedule sessions finish work they announced
description: When an unattended turn ends on an announcement without doing the work, the agent's own model reviews it and the turn continues, within a budget.
date: 2026-11-01
release: "0.51.0"
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api, cli]
links:
  docs: [/features/agent-triggers/#completion-check, /event-reference/#turncompletion_checked]
---

A trigger or schedule session could end a turn on "Submitting the review now"
without making the call. Nobody was watching, so the work was lost. Now, when a
tool-using turn in such a session is about to finish, the agent's own model
reviews whether the request was done. If it was not, an automatic continuation
joins the same turn and the agent carries on.

The continuation is labeled as automatic, never as the person's words, and is
bounded per input in turns, tokens and time. Each review records a
[`turn.completion_checked`](/event-reference/#turncompletion_checked) event. Worker restarts do not run a check twice or add
a second continuation.

The new agent setting [`completion_check`](/features/agent-triggers/#completion-check)
controls it: `unattended` (the
default), `always` to check chat too, or `off`:

```bash
everruns agents update agent_01h9 --completion-check always --reason 'Check chat turns too'
```
