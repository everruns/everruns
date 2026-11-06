---
title: "See human waiting time separately in traces"
description: "OpenTelemetry traces separate approval and question waits from agent work, preserving their timing and outcome when a worker restarts."
date: 2026-11-06
kind: improvement
significance: minor
products: [platform, framework]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

OpenTelemetry traces now show `human.wait` spans for tool approvals and questions
that wait for a person. A slow turn can be waiting for an answer rather than
running a tool; the separate interval makes that visible.

Each span records the wait's duration and outcome, including rejection, timeout
and cancellation. SQLite sessions retain those facts across restart; hosted
sessions recover them from their event journal. Unattended answers do not count
as human waiting time.

Wait spans contain no prompts, answers, credentials or URLs. They use the existing
[OpenTelemetry configuration](/sre/environment-variables/). Already parked SQLite
sessions created before this change begin collecting wait facts on their next
request; old durations are not guessed.

For example, a question that reaches its deadline appears as:

```json
{
  "name": "human.wait",
  "everruns.human_wait.kind": "question",
  "everruns.human_wait.outcome": "timeout",
  "everruns.human_wait.duration_ms": 6000
}
```
