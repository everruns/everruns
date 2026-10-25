---
title: Sandboxes are disposable, their records are kept
description: Paused Sandboxes are deleted after a day, a Delete idle action drops disposable ones at once, and every Sandbox record is now kept forever.
date: 2026-10-25
kind: improvement
significance: minor
products: [platform]
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4915]
---

Paused Sandboxes used to stay with the provider until their Session was deleted, which never
happens for Sessions started by triggers. Everruns now deletes a paused Sandbox after one day
without use. Pick 3, 7, or 30 days, or **Never**, under **Delete after paused** on the
[Sandbox Template](/features/sandbox-templates/), or set **When idle** to **Delete** to drop
it as soon as it goes idle. The Session stays; its next run gets a fresh Sandbox.

```json
{
  "lifecycle": {
    "idle_after_seconds": 900,
    "delete_after_seconds": 604800
  }
}
```

Set `delete_after_seconds` to `null` to keep paused Sandboxes.

Only the Sandbox at the provider is deleted. Its record and history stay on the **Sandboxes**
page under **Deleted**, and deleted records are no longer purged after 30 days, so
`SANDBOX_HISTORY_RETENTION_DAYS` is gone.

A Sandbox whose Session is stuck mid-turn is now paused after six hours without use, instead of
running until someone stops it.
