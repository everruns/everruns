---
title: Coordinator
description: Make an agent behave as a coordinator. It hands work to threads, follows them, and reports back, instead of doing the work itself.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `coordinator` |
| **Category** | Orchestration |
| **Dependencies** | [Threads](/capabilities/threads/) |

Coordinator is behaviour only: a prompt that makes the agent route work. It
starts one thread per piece of work, sends follow-ups to the thread that owns
the work, tells the person in a line when a thread finishes or asks, and
closes threads the person accepts. It adds no tools; it needs
[Threads](/capabilities/threads/) for those.

```json
[
  { "ref": "threads", "config": { "self": true, "max_open": 4 } },
  { "ref": "coordinator" }
]
```

Platform Chat is a coordinator: it has Threads (running itself or any agent)
and Coordinator.
