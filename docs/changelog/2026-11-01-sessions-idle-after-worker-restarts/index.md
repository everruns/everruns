---
title: Sessions go idle after their turn when workers restart
description: A session whose turn finished while rolling deploys restarted its workers no longer stays active, and sessions already stuck that way are resumed and idled.
date: 2026-11-01
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/events/]
---

A turn that finished across several worker restarts could leave its session
`active` for days: the turn's answer was in the log, but `session.idled` never
followed, so the session showed as running and nothing ran it again. Webhook
and trigger sessions hit this during rolling deploys.

The worker that takes over a finished turn now idles its session even when the
turn already used up its restarts. A server sweep also resumes any session left
`active` with no worker on it, so sessions already stuck this way settle on
their own within minutes of the update:

```bash
everruns sessions get session_01h9 | jq '{status, activity, last_turn_status}'
# before: { "status": "active", "activity": "running", "last_turn_status": "completed" }
# after:  { "status": "idle", "activity": "completed", "last_turn_status": "completed" }
```

See [Events](/features/events/) for `session.idled` and the other session
lifecycle events.
