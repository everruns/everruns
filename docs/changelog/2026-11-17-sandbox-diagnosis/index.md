---
title: Ask why a Sandbox stopped, and get an answer
description: Sandboxes keep a log of who paused, resumed or deleted them and why, and a diagnose command joins it with deadlines, queued work and the provider's view.
date: 2026-11-17
kind: feature
significance: minor
products: [platform]
surfaces: [api, cli]
links:
  docs: [/features/sandbox-templates/]
---

A Sandbox that paused overnight used to leave only a state change behind. Now the idle timer, the
idle sweep, the retention sweep and people acting through the API each record why they paused,
resumed or deleted a Sandbox, and why they left it alone when the session was mid-turn.

`everruns sandboxes diagnose` puts that log beside the deadlines its pinned lifecycle sets, the
session's queued and running work and, for organization admins, what Daytona, E2B, Modal, boxd or
Kubernetes reports right now. It answers with findings and a fix for each, such as a provider
auto-stop that fires before **Idle after**:

```bash
everruns sandboxes diagnose sandbox_01h9
everruns sandboxes events list sandbox_01h9 --from 2026-11-09T02:00:00Z
```

Platform Chat runs the same commands when you ask it. See
[Why did my Sandbox stop?](/features/sandbox-templates/#why-did-my-sandbox-stop).
