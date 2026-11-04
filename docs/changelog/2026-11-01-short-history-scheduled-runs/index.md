---
title: "Scheduled runs no longer send the whole session to the model"
description: "A recurring schedule's run now sees its task, the latest summary and its own recent runs, so a schedule in a long session stays cheap on every fire."
date: 2026-11-01
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/capabilities/session-schedules/]
---

**For agents:** each run of a recurring session schedule now sends the model a
short history: the latest conversation summary, the last 10 messages of the
same schedule's earlier runs, and the run itself. The rest of the session stays
stored but is left out of the prompt. A poll in a session of thousands of
messages used to resend all of them on every run; now its cost stays flat.

```text
system prompt
latest conversation summary
run 7: "Poll the claim" → "still pending"
run 8: "Poll the claim" → "still pending"
run 9: "Poll the claim" (this run)
```

The schedule tools ask the agent for a self-contained description, since a run
no longer sees the conversation that created it. One-shot schedules and
ordinary turns keep the full history. See
[Schedules](/capabilities/session-schedules/).
