---
title: Watch what an agent did on its Sandbox desktop
description: Daytona desktops are recorded as video. Each turn's recording is saved to the Session's files and plays in the chat under the turn.
date: 2026-10-26
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
---

A Daytona Sandbox Template with a desktop now records it. Recording starts when the agent first
uses the desktop in a turn and stops when the Session goes idle. The video is saved read-only to
`/recordings/` in the Session's files, so it stays after the Sandbox is deleted. The chat shows it
under the turn, with a player and a download link. To turn recording off, set `record` to `false`:

```json
{
  "target": {
    "kind": "managed",
    "provider": "daytona",
    "options": {
      "desktop": { "record": false }
    }
  }
}
```

E2B desktops are not recorded yet. See [Sandbox Templates](/features/sandbox-templates/#desktop).
